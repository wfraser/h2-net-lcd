use anyhow::{Context, Result};
pub use lcd::Display;
use lcd::{
    DisplayBlink,
    DisplayCursor,
    DisplayMode,
    FunctionDots,
    FunctionLine,
};
use lcd_pcf8574::Pcf8574;

const I2C_BUS: u8 = 2;
const I2C_ADDR: u16 = 0x27;

pub fn init_display() -> Result<Display<Pcf8574>> {
    let mut display = Display::new(
        Pcf8574::new(I2C_BUS, I2C_ADDR)
            .context("failed to open I2C device")?);

    display.init(FunctionLine::Line2, FunctionDots::Dots5x8);
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
