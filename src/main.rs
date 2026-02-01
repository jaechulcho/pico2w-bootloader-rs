#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::gpio;
use embassy_time::{Duration, Timer};
use gpio::{Level, Output};
use {defmt_rtt as _, panic_probe as _};

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    // Use PIN_28 for LED as seen in pico2w-shell-rs
    let mut led = Output::new(p.PIN_28, Level::Low);

    loop {
        info!("led on!");
        led.set_high();
        Timer::after(Duration::from_millis(500)).await;

        info!("led off!");
        led.set_low();
        Timer::after(Duration::from_millis(500)).await;
    }
}

/// Program metadata for `picotool info`
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [embassy_rp::binary_info::EntryAddr; 4] = [
    embassy_rp::binary_info::rp_program_name!(c"pico2w-bootloader-rs"),
    embassy_rp::binary_info::rp_cargo_version!(),
    embassy_rp::binary_info::rp_program_description!(c"Pico 2 W Rust Bootloader"),
    embassy_rp::binary_info::rp_program_build_attribute!(),
];
