//! Blinks an LED
//!
//! This assumes that LD2 (blue) is connected to pb7 and LD3 (red) is connected
//! to pb14. This assumption is true for the nucleo-h743zi board.

#![no_std]
#![no_main]
use satrs_stm32h7_nucleo_rtic as _;

use stm32h7xx_hal::{block, prelude::*, timer::Timer};

use cortex_m_rt::entry;

#[entry]
fn main() -> ! {
    defmt::println!("starting stm32h7 blinky example");

    // Get access to the device specific peripherals from the peripheral access crate
    let dp = stm32h7xx_hal::stm32::Peripherals::take().unwrap();

    // Take ownership over the RCC devices and convert them into the corresponding HAL structs
    let rcc = dp.RCC.constrain();

    let pwr = dp.PWR.constrain();
    let pwrcfg = pwr.freeze();

    // Freeze the configuration of all the clocks in the system and
    // retrieve the Core Clock Distribution and Reset (CCDR) object
    let rcc = rcc.use_hse(8.MHz()).bypass_hse();
    let ccdr = rcc.freeze(pwrcfg, &dp.SYSCFG);

    // Acquire the GPIOB peripheral
    let gpiob = dp.GPIOB.split(ccdr.peripheral.GPIOB);

    // Configure gpio B pin 0 as a push-pull output.
    let mut ld1 = gpiob.pb0.into_push_pull_output();

    // Configure gpio B pin 7 as a push-pull output.
    let mut ld2 = gpiob.pb7.into_push_pull_output();

    // Configure gpio B pin 14 as a push-pull output.
    let mut ld3 = gpiob.pb14.into_push_pull_output();

    // Configure the timer to trigger an update every second
    let mut timer = Timer::tim1(dp.TIM1, ccdr.peripheral.TIM1, &ccdr.clocks);
    timer.start(1.Hz());

    // Wait for the timer to trigger an update and change the state of the LED
    loop {
        ld1.toggle();
        ld2.toggle();
        ld3.toggle();
        block!(timer.wait()).unwrap();
    }
}
