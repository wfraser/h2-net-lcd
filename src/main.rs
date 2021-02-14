use anyhow::{Context, Result};
use std::collections::VecDeque;
use std::fmt::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use systemstat::{Platform, System};

// TODO: make this configurable
const NET_DEV_NAMES: [&str; 6] = [
    "ether0", "ether1", "ether2", "ether3", "ether4", "ether5",
];

#[cfg(not(feature = "mock"))]
mod lcd_display;

#[cfg(not(feature = "mock"))]
use lcd_display::{init_display, stop_display};

#[cfg(feature = "mock")]
mod mock_display;

#[cfg(feature = "mock")]
use mock_display::{init_display, stop_display};

struct NetStats {
    name: String,
    last: (Instant, u64, u64),
    pub buckets: VecDeque<(Instant, f64, f64)>,
}

impl NetStats {
    pub fn new(name: String) -> Result<Self> {
        let last = Self::sample(&name)?;
        Ok(Self {
            name,
            last,
            buckets: VecDeque::new(),
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
        let (now, new_rx, new_tx) = Self::sample(&self.name)?;
        let (prev, old_rx, old_tx) = self.last;
        let dur = (now - prev).as_secs_f64();
        let rx_mbps = (new_rx - old_rx) as f64 / dur * 8. / 1_000_000.;
        let tx_mbps = (new_tx - old_tx) as f64 / dur * 8. / 1_000_000.;
        self.last = (now, new_rx, new_tx);

        while let Some((time, _, _)) = self.buckets.front() {
            if (now - *time).as_secs_f64() < 60. {
                break;
            }
            self.buckets.pop_front();
        }
        self.buckets.push_back((now, rx_mbps, tx_mbps));

        Ok((rx_mbps.ceil() as u16, tx_mbps.ceil() as u16))
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

    fn get_load(&mut self) -> Result<Vec<f64>> {
        let last = std::mem::replace(
            &mut self.last,
            System::new().cpu_load().context("failed to get CPU load")?);
        let meas = last.done().context("failed to update CPU load measurement")?;
        let mut result = vec![];
        for core in meas {
            result.push(1. - core.idle as f64);
        }
        Ok(result)
    }
}

fn avail_mem_mib() -> Result<(u64, u64)> {
    let mem = System::new().memory()?;
    let total = mem.total.as_u64() / 1_048_576;
    let avail = mem.platform_memory.meminfo.get("MemAvailable").unwrap().as_u64() / 1_048_576;
    Ok((avail, total))
}

fn display_char(value: f64, row: u8) -> u8 {
    assert!(value >= 0.);
    assert!(value <= 1.);
    assert!(row < 3);
    // we've got 3 rows each 8 pixels high, so 24 values
    let quantized = (value * 24.).ceil() as u8;
    let row = 2 - row;
    let pixels = match (quantized / 8).cmp(&row) {
        std::cmp::Ordering::Greater => 8,
        std::cmp::Ordering::Less => 0,
        std::cmp::Ordering::Equal => quantized - (8 * row)
    };

    if pixels == 0 {
        // zero pixels is a space char
        b' '
    } else {
        // otherwise it's in custom chars 0 thru 7
        pixels - 1
    }
}

#[cfg(test)]
#[test]
fn test_display_char() {
    assert_eq!(32, display_char(0., 0));
    assert_eq!(32, display_char(0., 1));
    assert_eq!(32, display_char(0., 2));

    assert_eq!(7, display_char(1., 0));
    assert_eq!(7, display_char(1., 1));
    assert_eq!(7, display_char(1., 2));

    assert_eq!(32, display_char(0.5, 0));
    assert_eq!(3, display_char(0.5, 1));
    assert_eq!(7, display_char(0.5, 2));

    assert_eq!(32, display_char(0.666, 0));
    assert_eq!(7, display_char(0.666, 1));
    assert_eq!(7, display_char(0.666, 2));
}

fn main() -> Result<()> {
    let mut display = init_display()?;

    let stop = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, stop.clone())
        .context("failed to set SIGTERM handler")?;
    signal_hook::flag::register(signal_hook::consts::SIGINT, stop.clone())
        .context("failed to set SIGINT handler")?;

    let mut ifstats = vec![];
    for &name in &NET_DEV_NAMES {
        ifstats.push(NetStats::new(name.to_owned())?);
    }

    let mut cpustats = CPUStats::new()?;

    while !stop.load(Ordering::SeqCst) {

        let cpu = cpustats.get_load()?;

        let mut mbps = vec![];
        for dev in ifstats.iter_mut() {
            mbps.push(dev.mbps()?);
        }

        let (mem_avail, mem_total) = avail_mem_mib()
            .context("failed to get available memory")?;
        let mem = (mem_total - mem_avail) as f64 / mem_total as f64;

        let temperature = System::new().cpu_temp()
            .context("failed to get CPU temperature")?;

        for row in 0 .. 3 {
            display.position(0, row);

            for &core in &cpu {
                display.write(display_char(core, row));
            }

            display.write(b'|');

            for &(rx, tx) in &mbps {
                let rx = (rx as f64 / 1000.).clamp(0., 1.);
                let tx = (tx as f64 / 1000.).clamp(0., 1.);
                display.write(display_char(tx, row));
                display.write(display_char(rx, row));
            }

            display.print("| ");

            display.write(display_char(mem, row));
        }

        display.position(0, 3);
        display.print("cpu ");
        write!(&mut display, "{:>2}", temperature.round())?;
        display.write(0xdf); // degree sign
        display.print("C ");

        let max_rx_mbps = ifstats
            .iter()
            .flat_map(|netstats| netstats.buckets.iter())
            .map(|(_, rx, _)| rx.ceil() as u16)
            .max()
            .unwrap();
        let max_tx_mbps = ifstats
            .iter()
            .flat_map(|netstats| netstats.buckets.iter())
            .map(|(_, _, tx)| tx.ceil() as u16)
            .max()
            .unwrap();
        write!(&mut display, "{:>3}/{:>3}", max_tx_mbps, max_rx_mbps)?;

        display.print(" mem");

        #[cfg(feature = "mock")]
        {
            print!("\x1b[2J");
            println!("____________________");
            display.dump();
            println!("____________________");
        }
        thread::sleep(Duration::from_millis(500));
    }

    stop_display(display);
    Ok(())
}
