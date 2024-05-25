#![no_std]
#![no_main]
use satrs_stm32f3_disco_rtic as _;

use stm32f3_discovery::leds::Leds;
use stm32f3_discovery::stm32f3xx_hal::delay::Delay;
use stm32f3_discovery::stm32f3xx_hal::{pac, prelude::*};
use stm32f3_discovery::switch_hal::{OutputSwitch, ToggleableOutputSwitch};

#[cortex_m_rt::entry]
fn main() -> ! {
    defmt::println!("STM32F3 Discovery Blinky");
    let dp = pac::Peripherals::take().unwrap();
    let mut rcc = dp.RCC.constrain();
    let cp = cortex_m::Peripherals::take().unwrap();
    let mut flash = dp.FLASH.constrain();
    let clocks = rcc.cfgr.freeze(&mut flash.acr);
    let mut delay = Delay::new(cp.SYST, clocks);

    let mut gpioe = dp.GPIOE.split(&mut rcc.ahb);
    let mut leds = Leds::new(
        gpioe.pe8,
        gpioe.pe9,
        gpioe.pe10,
        gpioe.pe11,
        gpioe.pe12,
        gpioe.pe13,
        gpioe.pe14,
        gpioe.pe15,
        &mut gpioe.moder,
        &mut gpioe.otyper,
    );
    let delay_ms = 200u16;
    loop {
        leds.ld3_n.toggle().ok();
        delay.delay_ms(delay_ms);
        leds.ld3_n.toggle().ok();
        delay.delay_ms(delay_ms);

        //explicit on/off
        leds.ld4_nw.on().ok();
        delay.delay_ms(delay_ms);
        leds.ld4_nw.off().ok();
        delay.delay_ms(delay_ms);

        leds.ld5_ne.on().ok();
        delay.delay_ms(delay_ms);
        leds.ld5_ne.off().ok();
        delay.delay_ms(delay_ms);

        leds.ld6_w.on().ok();
        delay.delay_ms(delay_ms);
        leds.ld6_w.off().ok();
        delay.delay_ms(delay_ms);

        leds.ld7_e.on().ok();
        delay.delay_ms(delay_ms);
        leds.ld7_e.off().ok();
        delay.delay_ms(delay_ms);

        leds.ld8_sw.on().ok();
        delay.delay_ms(delay_ms);
        leds.ld8_sw.off().ok();
        delay.delay_ms(delay_ms);

        leds.ld9_se.on().ok();
        delay.delay_ms(delay_ms);
        leds.ld9_se.off().ok();
        delay.delay_ms(delay_ms);

        leds.ld10_s.on().ok();
        delay.delay_ms(delay_ms);
        leds.ld10_s.off().ok();
        delay.delay_ms(delay_ms);
    }
}
