use anyhow::{Context, Result};
use lcd::{
    Direction,
    Display,
    DisplayBlink,
    DisplayCursor,
    DisplayMode,
    FunctionDots,
    FunctionLine,
};
use lcd_pcf8574::Pcf8574;
use std::fmt::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use systemstat::{NetworkStats, Platform, System};

const I2C_BUS: u8 = 1;
const I2C_ADDR: u16 = 0x27;

struct NetStats {
    name: String,
    last: (Instant, u64, u64),
}

impl NetStats {
    pub fn new(name: String) -> Result<Self> {
        let last = Self::sample(&name)?;
        Ok(Self {
            name,
            last,
        })
    }

    fn sample(name: &str) -> Result<(Instant, u64, u64)> {
        let stats = System::new().network_stats(name)
            .with_context(|| format!("failed to get stats for {}", name))?;
        let now = Instant::now();
        let rx_bytes = stats.rx_bytes.as_u64();
        let tx_bytes = stats.tx_bytes.as_u64();
        Ok((now, rx_bytes, tx_bytes))
    }

    pub fn mbps(&mut self) -> Result<(u16, u16)> {
        let new = Self::sample(&self.name)?;
        let dur = (new.0 - self.last.0).as_secs_f64();
        let rx = (new.1 - self.last.1) as f64 / dur / 1_000_000.;
        let tx = (new.2 - self.last.2) as f64 / dur / 1_000_000.;
        self.last = new;
        Ok((rx.ceil() as u16, tx.ceil() as u16))
    }
}

fn ifstats() -> Result<Vec<NetStats>> {
    let sys = System::new();
    let mut names: Vec<String> = vec![];
    let networks = sys.networks()
        .context("failed to get network interfaces")?;
    for (name, _) in networks {
        if name.starts_with("eth") || name.starts_with("wlan") {
            names.push(name);
        }
    }
    names.sort();
    let mut result = vec![];
    for name in names.into_iter() {
        result.push(NetStats::new(name)?);
    }
    Ok(result)
}

struct CPUStats {
    last: systemstat::DelayedMeasurement<Vec<systemstat::CPULoad>>
}

impl CPUStats {
    pub fn new() -> Result<Self> {
        Ok(Self {
            last: System::new().cpu_load().context("failed to get CPU load")?,
        })
    }

    fn get_load(&mut self) -> Result<Vec<u8>> {
        let last = std::mem::replace(
            &mut self.last,
            System::new().cpu_load().context("failed to get CPU load")?);
        let meas = last.done().context("failed to update CPU load measurement")?;
        println!("{:#?}", meas.len());
        let mut result = vec![];
        for core in meas {
            result.push(((1. - core.idle) * 100.).ceil() as u8);
        }
        Ok(result)
    }
}

fn main() -> Result<()> {
    let mut display = Display::new(
        Pcf8574::new(I2C_BUS, I2C_ADDR).context("failed to open I2C device")?);
    display.init(FunctionLine::Line2, FunctionDots::Dots5x8);
    display.display(
        DisplayMode::DisplayOn,
        DisplayCursor::CursorOff,
        DisplayBlink::BlinkOff);

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        ctrlc::set_handler(
            move || {
                stop.store(true, Ordering::SeqCst);
            })
            .context("failed to set SIGINT handler")?;
    }

    let mut ifstats = ifstats()?;
    let mut cpustats = CPUStats::new()?;

    while !stop.load(Ordering::SeqCst) {
        display.position(0, 0);
        for (i, dev) in ifstats.iter_mut().take(5).enumerate() {
            let i = i as u8;
            let (rx, tx) = dev.mbps()?;
            display.position(i * 4, 0);
            write!(&mut display, "e{}", i).unwrap();
            display.position(i * 4, 1);
            write!(&mut display, "{:>3}", tx).unwrap();
            display.position(i * 4, 2);
            write!(&mut display, "{:>3}", rx).unwrap();
        }
        display.position(0, 3);
        display.print("cpu ");
        for core in cpustats.get_load()? {
            write!(&mut display, "{:>2} ", core).unwrap();
        }
        thread::sleep(Duration::from_millis(500));
    }

    display.display(
        DisplayMode::DisplayOff,
        DisplayCursor::CursorOff,
        DisplayBlink::BlinkOff);
    display.unwrap().backlight(false);
    Ok(())
}
