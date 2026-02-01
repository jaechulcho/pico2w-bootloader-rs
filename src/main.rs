#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_futures::select::{select, Either};
use embassy_rp::flash::{Async, Flash};
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, DMA_CH1, DMA_CH2, FLASH, UART0};
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
const METADATA_SIZE: u32 = 256; // One flash page
const REAL_APP_BASE: u32 = APP_BASE + METADATA_SIZE;

const MAGIC_APPS: &[u8; 4] = b"APPS";

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
    let mut flash: Flash<FLASH, Async, { 2 * 1024 * 1024 }> = Flash::new(p.FLASH, p.DMA_CH2, Irqs);

    let mut uart_buf = [0u8; 1];

    // Check application health (Magic + CRC32)
    let app_healthy = unsafe { is_app_healthy(APP_BASE) };
    let mut update_mode = !app_healthy;

    if update_mode {
        warn!("Application is corrupted or missing! Entering Update Mode.");
    } else {
        info!("Application healthy. Press 'u' for Update, or wait 3s to Jump...");
        let start_time = embassy_time::Instant::now();
        let timeout = Duration::from_secs(3);
        loop {
            let elapsed = start_time.elapsed();
            if elapsed >= timeout {
                info!("Timeout, jumping to app...");
                break;
            }
            let remaining = timeout - elapsed;
            match select(Timer::after(remaining), uart.read(&mut uart_buf)).await {
                Either::First(_) => {
                    info!("Timeout, jumping to app...");
                    break;
                }
                Either::Second(res) => {
                    if res.is_ok() {
                        let c = uart_buf[0];
                        if c == b'u' || c == b'p' {
                            update_mode = true;
                            info!("Entering Update Mode!");
                            break;
                        } else {
                            // Ignore other characters (like trailing 'reboot\r\n' junk)
                            debug!("Ignored byte during wait: 0x{:02x}", c);
                        }
                    }
                }
            }
        }
    }

    if update_mode {
        led.set_low();
        info!("DFU Mode: Wait for magic 0xAA...");
        
        loop {
            if let Ok(_) = uart.read(&mut uart_buf).await {
                if uart_buf[0] == 0xAA {
                    break;
                }
            }
        }

        let mut header = [0u8; 8]; // [Length(4) | CRC32(4)]
        if uart.read(&mut header).await.is_ok() {
            let len = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
            let crc_val = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
            info!("Receiving {} bytes, CRC32: 0x{:x}", len, crc_val);

            // Erase app area + metadata
            let total_erase = (len + METADATA_SIZE + 4095) & !4095;
            info!("Erasing {} bytes...", total_erase);
            if let Err(e) = flash.erase(APP_OFFSET, APP_OFFSET + total_erase) {
                error!("Erase failed: {:?}", e);
            } else {
                // Send ACK ONLY AFTER successful erase.
                // This prevents the downloader from timing out while we are busy erasing.
                let _ = uart.write(&[0x06]).await;

                let mut write_buf = [0u8; 4096];
                let mut received = 0;
                
                // Real app writing
                while received < len {
                    let chunk_len = core::cmp::min(4096, (len - received) as usize);
                    if uart.read(&mut write_buf[..chunk_len]).await.is_ok() {
                        if let Err(e) = flash.write(APP_OFFSET + METADATA_SIZE + received, &write_buf[..chunk_len]) {
                            error!("Flash write failed: {:?}", e);
                            break;
                        }
                        received += chunk_len as u32;
                        info!("Received {}/{} bytes", received, len);
                        
                        // Send ACK for each chunk
                        let _ = uart.write(&[0x06]).await;
                    } else {
                        error!("UART read failed");
                        break;
                    }
                }

                if received == len {
                    info!("Verifying CRC32...");
                    if unsafe { verify_flash_crc(APP_BASE + METADATA_SIZE, len, crc_val) } {
                        info!("CRC OK! Writing metadata...");
                        let mut metadata = [0u8; 256];
                        metadata[0..4].copy_from_slice(MAGIC_APPS);
                        metadata[4..8].copy_from_slice(&len.to_le_bytes());
                        metadata[8..12].copy_from_slice(&crc_val.to_le_bytes());
                        
                        if let Err(e) = flash.write(APP_OFFSET, &metadata) {
                            error!("Metadata write failed: {:?}", e);
                        } else {
                            info!("Update complete! Resetting system...");
                            // Wait a bit for the message to be sent
                            for _ in 0..100000 { core::hint::spin_loop(); }
                            cortex_m::peripheral::SCB::sys_reset();
                        }
                    } else {
                        error!("CRC mismatch! Application might be corrupted.");
                    }
                }
            }
        }
    }

    led.set_low();
    info!("Jumping to app at 0x{:x}...", REAL_APP_BASE);
    for _ in 0..100000 { core::hint::spin_loop(); }
    unsafe {
        jump_to_app(REAL_APP_BASE);
    }
}

unsafe fn is_app_healthy(address: u32) -> bool {
    let magic = unsafe { core::slice::from_raw_parts(address as *const u8, 4) };
    if magic != MAGIC_APPS {
        return false;
    }

    let len = unsafe { *((address + 4) as *const u32) };
    let expected_crc = unsafe { *((address + 8) as *const u32) };

    info!("App Metadata: Len={}, CRC=0x{:x}", len, expected_crc);

    // Basic SP check within the app first 4 bytes
    let sp = unsafe { *((address + METADATA_SIZE) as *const u32) };
    if sp < 0x20000000 || sp > 0x20082000 {
        return false;
    }

    unsafe { verify_flash_crc(address + METADATA_SIZE, len, expected_crc) }
}

unsafe fn verify_flash_crc(address: u32, len: u32, expected: u32) -> bool {
    let data = unsafe { core::slice::from_raw_parts(address as *const u8, len as usize) };
    let crc = crc::Crc::<u32>::new(&crc::CRC_32_ISO_HDLC);
    let mut digest = crc.digest();
    digest.update(data);
    let calculated = digest.finalize();
    
    info!("CRC Check: Calc=0x{:x}, Exp=0x{:x}", calculated, expected);
    calculated == expected
}

unsafe fn jump_to_app(address: u32) -> ! {
    let (sp, reset_handler) = unsafe {
        let sp = *(address as *const u32);
        let reset_handler = *((address + 4) as *const u32);
        (sp, reset_handler)
    };

    if sp < 0x20000000 || sp > 0x20082000 || reset_handler < 0x10000000 {
        error!("Fatal: Invalid App at 0x{:x}!", address);
        loop { core::hint::spin_loop(); }
    }

    unsafe {
        let p = cortex_m::Peripherals::steal();
        p.SCB.vtor.write(address);
        cortex_m::asm::bootstrap(sp as *const u32, reset_handler as *const u32);
    }
}

#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [embassy_rp::binary_info::EntryAddr; 4] = [
    embassy_rp::binary_info::rp_program_name!(c"pico2w-bootloader-rs"),
    embassy_rp::binary_info::rp_cargo_version!(),
    embassy_rp::binary_info::rp_program_description!(c"Pico 2 W Rust Bootloader"),
    embassy_rp::binary_info::rp_program_build_attribute!(),
];
