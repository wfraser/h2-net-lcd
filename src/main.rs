use anyhow::{Context, Result};
use lcd::{
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
use systemstat::{Platform, System};

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
        let rx = (new.1 - self.last.1) as f64 / dur * 8. / 1_000_000.;
        let tx = (new.2 - self.last.2) as f64 / dur * 8. / 1_000_000.;
        self.last = new;
        Ok((rx.ceil() as u16, tx.ceil() as u16))
    }
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
        let mut result = vec![];
        for core in meas {
            result.push(((1. - core.idle) * 100.).ceil() as u8);
        }
        Ok(result)
    }
}

#[cfg(feature = "mock")]
struct MockDisplay {
    lines: Vec<Vec<u8>>,
    pos: (usize, usize),
}

#[cfg(feature = "mock")]
impl MockDisplay {
    pub fn new() -> Self {
        Self {
            lines: vec![vec![b' '; 20]; 4],
            pos: (0, 0),
        }
    }

    pub fn position(&mut self, col: u8, row: u8) {
        self.pos = (row.min(3) as usize, col.min(19) as usize);
    }

    pub fn print(&mut self, s: &str) {
        for byte in s.as_bytes() {
            self.lines[self.pos.0][self.pos.1] = *byte;
            self.pos.1 += 1;
            if self.pos.1 == 20 {
                self.pos.0 += 1;
                self.pos.1 = 0;
            }
            if self.pos.0 == 4 {
                self.pos.0 = 0;
            }
        }
    }

    pub fn dump(&self) {
        for line in &self.lines {
            println!("{}", String::from_utf8_lossy(line));
        }
    }
}

#[cfg(feature = "mock")]
impl std::fmt::Write for MockDisplay {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.print(s);
        Ok(())
    }
}

fn main() -> Result<()> {
    #[cfg(not(feature = "mock"))]
    let mut display = {
        let mut display = Display::new(
            Pcf8574::new(I2C_BUS, I2C_ADDR).context("failed to open I2C device")?);
        display.init(FunctionLine::Line2, FunctionDots::Dots5x8);
        display.display(
            DisplayMode::DisplayOn,
            DisplayCursor::CursorOff,
            DisplayBlink::BlinkOff);
        display
    };

    #[cfg(feature = "mock")]
    let mut display = MockDisplay::new();

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        ctrlc::set_handler(
            move || {
                stop.store(true, Ordering::SeqCst);
            })
            .context("failed to set SIGINT handler")?;
    }

    let mut ifstats = vec![
        NetStats::new("ether0".to_owned())?,
        NetStats::new("ether0.201".to_owned())?,
        NetStats::new("ppp0".to_owned())?,
        NetStats::new("ether1".to_owned())?,
    ];
    let mut cpustats = CPUStats::new()?;

    while !stop.load(Ordering::SeqCst) {
        display.position(0, 0);
        for (i, dev) in ifstats.iter_mut().take(5).enumerate() {
            let i = i as u8;
            let (rx, tx) = dev.mbps()?;
            display.position(i * 4, 0);
            write!(&mut display, "{:>3}", tx)?;
            display.position(i * 4, 1);
            write!(&mut display, "{:>3}", rx)?;
        }
        display.position(0, 2);
        display.print("cpu ");
        for core in cpustats.get_load()? {
            write!(&mut display, "{:>2} ", core.min(99))?;
        }

        let temp = System::new().cpu_temp()
            .context("failed to get CPU temperature")?;
        write!(&mut display, "{:>2}C", temp)?;

        #[cfg(feature = "mock")]
        {
            print!("\x1b[2J");
            println!("____________________");
            display.dump();
            println!("____________________");
        }
        thread::sleep(Duration::from_millis(500));
    }

    #[cfg(not(feature = "mock"))]
    {
        display.display(
            DisplayMode::DisplayOff,
            DisplayCursor::CursorOff,
            DisplayBlink::BlinkOff);
        display.unwrap().backlight(false);
    }
    Ok(())
}
