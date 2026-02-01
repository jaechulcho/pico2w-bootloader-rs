#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_futures::select::{select, Either};
use embassy_rp::flash::{Async, Flash};
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{FLASH, UART0, DMA_CH0, DMA_CH1, DMA_CH2};
use embassy_rp::uart::{Config as UartConfig, InterruptHandler as UartInterruptHandler, Uart};
use embassy_rp::{bind_interrupts, dma};
use embassy_time::{Duration, Timer};
use embedded_storage::nor_flash::NorFlash;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    UART0_IRQ => UartInterruptHandler<UART0>;
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>,
                 dma::InterruptHandler<DMA_CH1>,
                 dma::InterruptHandler<DMA_CH2>;
});

const APP_OFFSET: u32 = 64 * 1024; // 64KB
const FLASH_BASE_ADDR: u32 = 0x1000_0000;
const APP_BASE: u32 = FLASH_BASE_ADDR + APP_OFFSET;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let mut led = Output::new(p.PIN_28, Level::Low);

    // Initial UART setup for status/DFU
    let mut uart_config = UartConfig::default();
    uart_config.baudrate = 115200;
    let mut uart = Uart::new(p.UART0, p.PIN_0, p.PIN_1, Irqs, p.DMA_CH0, p.DMA_CH1, uart_config);

    info!("pico2w-bootloader-rs starting...");
    led.set_high();

    // Flash driver setup
    // On RP2350 with newest embassy-rp, Flash::new takes (flash, dma, irq)
    // The irq is the DMA interrupt binding.
    let mut flash: Flash<FLASH, Async, { 2 * 1024 * 1024 }> = Flash::new(p.FLASH, p.DMA_CH2, Irqs);

    let mut uart_buf = [0u8; 1];
    info!("Press 'u' for Update, or wait 3s to Jump...");
    
    // Simple 3s wait for 'u' key
    let mut update_mode = false;
    
    match select(Timer::after(Duration::from_secs(3)), uart.read(&mut uart_buf)).await {
        Either::First(_) => {
            info!("Timeout, jumping to app...");
        }
        Either::Second(res) => {
            if res.is_ok() && uart_buf[0] == b'u' {
                update_mode = true;
                info!("Entering Update Mode!");
            }
        }
    }

    if update_mode {
        led.set_low();
        info!("Ready to receive application binary (Wait for magic byte 0xAA)...");
        
        loop {
            if let Ok(_) = uart.read(&mut uart_buf).await {
                if uart_buf[0] == 0xAA {
                    break;
                }
            }
        }

        let mut len_buf = [0u8; 4];
        if uart.read(&mut len_buf).await.is_ok() {
            let len = u32::from_le_bytes(len_buf);
            info!("Receiving {} bytes...", len);

            // Erase flash first
            let erase_len = (len + 4095) & !4095;
            info!("Erasing {} bytes at offset 0x{:x}...", erase_len, APP_OFFSET);
            if let Err(e) = flash.erase(APP_OFFSET, APP_OFFSET + erase_len) {
                error!("Flash erase failed: {:?}", e);
            } else {
                let mut write_buf = [0u8; 4096];
                let mut received = 0;
                while received < len {
                    let chunk_len = core::cmp::min(4096, (len - received) as usize);
                    if uart.read(&mut write_buf[..chunk_len]).await.is_ok() {
                        if let Err(e) = flash.write(APP_OFFSET + received, &write_buf[..chunk_len]) {
                            error!("Flash write failed at offset 0x{:x}: {:?}", APP_OFFSET + received, e);
                            break;
                        }
                        received += chunk_len as u32;
                        info!("Received {}/{} bytes", received, len);
                    } else {
                        error!("UART read failed");
                        break;
                    }
                }
                info!("Flash write complete!");
            }
        }
    }

    led.set_low();
    info!("Jumping to application at 0x{:x}...", APP_BASE);
    // Flush logs manually before jump
    for _ in 0..100000 {
        core::hint::spin_loop();
    }
    unsafe {
        jump_to_app(APP_BASE);
    }
}

unsafe fn jump_to_app(address: u32) -> ! {
    let (sp, reset_handler) = unsafe {
        let sp = *(address as *const u32);
        let reset_handler = *((address + 4) as *const u32);
        (sp, reset_handler)
    };

    info!("SP: 0x{:x}, ResetHandler: 0x{:x}", sp, reset_handler);

    // Basic validity check: SP should be in RAM
    if sp < 0x20000000 || sp > 0x20082000 {
        error!("Invalid Stack Pointer! Is the application flashed?");
        loop {
            core::hint::spin_loop();
        }
    }

    unsafe {
        let p = cortex_m::Peripherals::steal();
        p.SCB.vtor.write(address);
        
        // bootstrap(msp, rv)
        cortex_m::asm::bootstrap(sp as *const u32, reset_handler as *const u32);
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
