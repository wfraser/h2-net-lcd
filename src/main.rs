use anyhow::{Context, Result};
use lcd::{
    Direction,
    Display,
    DisplayBlink,
    DisplayCursor,
    DisplayMode,
};
use lcd_pcf8574::Pcf8574;
use std::fmt::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const I2C_BUS: u8 = 2;
const I2C_ADDR: u16 = 0x27;

fn main() -> Result<()> {
    let mut display = Display::new(
        Pcf8574::new(I2C_BUS, I2C_ADDR).context("failed to open I2C device")?);

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        ctrlc::set_handler(
            move || {
                stop.store(true, Ordering::SeqCst);
            })
            .context("failed to set SIGINT handler")?;
    }

    let mut iters = 0;
    while !stop.load(Ordering::SeqCst) {
        iters += 1;
        display.position(0, 0);
        write!(&mut display, "{}", iters).unwrap();
        display.scroll(Direction::Right);
        thread::sleep(Duration::from_millis(500));
    }

    display.display(
        DisplayMode::DisplayOff,
        DisplayCursor::CursorOff,
        DisplayBlink::BlinkOff);
    display.unwrap().backlight(false);
    Ok(())
}
