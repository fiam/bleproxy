//! bleproxy firmware for ESP32-C3 and ESP32-S3.
//!
//! Boots the chip and starts the Embassy executor on top of `esp-rtos`.
//! The BLE stack and the ESPHome API server will be spawned as tasks from
//! here; for now the firmware only announces itself on the serial console.

#![no_std]
#![no_main]
#![allow(
    clippy::future_not_send,
    reason = "Embassy tasks on the single-core executor are not Send by design"
)]

use embassy_executor::Spawner;
use esp_backtrace as _;
use esp_hal::timer::timg::TimerGroup;

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    esp_println::println!(
        "{} {} started",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    );

    // Nothing to spawn yet; the BLE and API server tasks will run here.
    let _ = spawner;
    loop {
        core::future::pending::<()>().await;
    }
}
