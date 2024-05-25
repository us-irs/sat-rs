#![no_main]
#![no_std]

use satrs_stm32h7_nucleo_rtic as _; // global logger + panicking-behavior + memory layout

#[cortex_m_rt::entry]
fn main() -> ! {
    defmt::println!("Hello, world!");

    satrs_stm32h7_nucleo_rtic::exit()
}
