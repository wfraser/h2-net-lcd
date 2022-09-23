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

const I2C_BUS: u8 = 2;
const I2C_BUS_FALLBACK: u8 = 1;
const I2C_ADDR: u16 = 0x27;

#[cfg(not(feature = "mock"))]
mod lcd_display;

#[cfg(not(feature = "mock"))]
use lcd_display::{init_display, stop_display, is_bus_fubar_error};

#[cfg(feature = "mock")]
mod mock_display;

#[cfg(feature = "mock")]
use mock_display::{init_display, stop_display, is_bus_fubar_error};

struct NetStats {
    name: String,
    last: NetSample,
    pub buckets: VecDeque<(Instant, NetSpeeds)>,
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

    fn sample(name: &str) -> Result<NetSample> {
        let stats = System::new().network_stats(name)
            .with_context(|| format!("failed to get stats for {}", name))?;
        let now = Instant::now();
        let rx_bytes = stats.rx_bytes.as_u64();
        let tx_bytes = stats.tx_bytes.as_u64();
        Ok(NetSample { time: now, rx_bytes, tx_bytes })
    }

    pub fn get_speeds(&mut self) -> Result<NetSpeeds> {
        let sample = Self::sample(&self.name)?;
        let now = sample.time;
        let speeds = sample.speeds(&self.last);
        self.last = sample;

        while let Some((time, _)) = self.buckets.front() {
            if (now - *time).as_secs_f64() < 60. {
                break;
            }
            self.buckets.pop_front();
        }
        self.buckets.push_back((now, speeds.clone()));

        Ok(speeds)
    }
}

#[derive(Debug, Clone)]
struct NetSpeed {
    bytes: u64,
    secs: f64,
}

impl NetSpeed {
    pub fn from_bytes(secs: f64, new: u64, old: u64) -> Self {
        let bytes = if new < old {
            // wrap-around
            u64::MAX - old + new
        } else {
            new - old
        };
        Self { bytes, secs }
    }

    pub fn mbps(&self) -> f64 {
        self.bytes as f64 / self.secs * 8. / 1_000_000.
    }

    #[allow(dead_code)]
    pub fn linear_display(&self) -> f64 {
        (self.mbps() / 1000.).clamp(0., 1.)
    }

    pub fn log_display(&self) -> f64 {
        (self.mbps().log10() / 3.).clamp(0., 1.)
    }
}

#[derive(Debug, Clone)]
struct NetSpeeds {
    tx: NetSpeed,
    rx: NetSpeed,
}

#[derive(Debug, Clone)]
struct NetSample {
    time: Instant,
    rx_bytes: u64,
    tx_bytes: u64,
}

impl NetSample {
    pub fn speeds(&self, last: &NetSample) -> NetSpeeds {
        let secs = (self.time - last.time).as_secs_f64();
        NetSpeeds {
            tx: NetSpeed::from_bytes(secs, self.tx_bytes, last.tx_bytes),
            rx: NetSpeed::from_bytes(secs, self.rx_bytes, last.rx_bytes),
        }
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
    let mut display = init_display(I2C_BUS, I2C_ADDR)
        .or_else(|e| {
            if is_bus_fubar_error(&e) {
                eprintln!("error on I2C bus {I2C_BUS}: {e}");
                eprintln!("trying I2C bus {I2C_BUS_FALLBACK} as fallback");
                match init_display(I2C_BUS_FALLBACK, I2C_ADDR) {
                    Err(e2) => {
                        eprintln!("I2C bus fallback also failed: {e2}");
                        Err(e) // return original error
                    }
                    Ok(d) => {
                        eprintln!("I2C bus fallback worked");
                        Ok(d)
                    }
                }
            } else {
                Err(e)
            }
        })?;

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

        let mut speeds = vec![];
        for dev in ifstats.iter_mut() {
            speeds.push(dev.get_speeds()?);
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

            for NetSpeeds { rx, tx } in &speeds {
                display.write(display_char(tx.log_display(), row));
                display.write(display_char(rx.log_display(), row));
            }

            display.print("| ");

            display.write(display_char(mem, row));
        }

        display.position(0, 3);
        display.print("cpu ");
        write!(&mut display, "{:>2}", temperature.round())?;
        display.write(0xdf); // degree sign
        display.print("C ");

        let mut max_rx_mbps = 0;
        let mut max_tx_mbps = 0;
        for dev in &ifstats {
            for (_time, NetSpeeds { rx, tx }) in &dev.buckets {
                max_rx_mbps = max_rx_mbps.max(rx.mbps().ceil() as u16);
                max_tx_mbps = max_tx_mbps.max(tx.mbps().ceil() as u16);
            }
        }
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
