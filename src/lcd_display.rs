use anyhow::{Context, Result};
use i2cdev::linux::LinuxI2CError;
pub use lcd::Display;
use lcd::{
    DisplayBlink,
    DisplayCursor,
    DisplayMode,
    FunctionDots,
    FunctionLine,
};
use lcd_pcf8574::{Pcf8574, ErrorHandling};
use nix::errno::Errno;
use std::cell::Cell;
use std::rc::Rc;

pub fn init_display(bus: u8, addr: u16) -> Result<Display<Pcf8574>> {
    let mut dev = Pcf8574::new(bus, addr)
        .context("failed to open I2C device")?;

    // For errors during init, save them into a Cell so we can return them, so callers can quickly
    // know if the parameters are wrong.
    let save_error = Rc::new(Cell::new(true));
    let error = Rc::new(Cell::new(Option::<anyhow::Error>::None));
    dev.on_error(ErrorHandling::Custom(Box::new({
        let save_error = Rc::clone(&save_error);
        let error = Rc::clone(&error);
        move |e| {
            if save_error.get() {
                error.set(Some(e.into()));
            } else {
                eprintln!("I/O error: {}", e);
            }
        }
    })));

    let mut display = Display::new(dev);
    display.init(FunctionLine::Line2, FunctionDots::Dots5x8);

    if let Some(e) = error.replace(None) {
        // Something went wrong during init, bail out now.
        return Err(e);
    }

    // If it successfully init'd, we're probably good to just print errors now.
    save_error.set(false);

    display.display(
        DisplayMode::DisplayOn,
        DisplayCursor::CursorOff,
        DisplayBlink::BlinkOff);

    // The display controller supports 8 custom characters. Characters are
    // 5 pixels wide by 8 pixels tall.
    // We'll use this to draw blocks of 8 different heights for our bar gauges.
    let mut bits = [0u8; 8]; // 8 bytes in array for 8 pixels tall
    for i in 0 .. 8 {
        bits[7 - i] = 0b11111; // 5 bits for 5 pixels wide
        display.upload_character(i as u8, bits);
    }

    Ok(display)
}

pub fn stop_display(mut display: Display<Pcf8574>) {
    display.display(
        DisplayMode::DisplayOff,
        DisplayCursor::CursorOff,
        DisplayBlink::BlinkOff);
    display.unwrap().backlight(false);
}

/// Is the given error indicative of the wrong I2C bus being used? (i.e. should you retry on a
/// different one?)
pub fn is_bus_fubar_error(e: &anyhow::Error) -> bool {
    if let Some(e) = e.downcast_ref::<LinuxI2CError>() {
        match e {
            LinuxI2CError::Io(e) => e.raw_os_error() == Some(libc::EREMOTEIO),
            LinuxI2CError::Nix(Errno::EREMOTEIO) => true,
            _ => false,
        }
    } else {
        false
    }
}
