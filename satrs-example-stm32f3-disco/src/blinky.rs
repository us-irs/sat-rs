#![no_std]
#![no_main]

extern crate panic_itm;

use cortex_m_rt::entry;

use stm32f3_discovery::stm32f3xx_hal::delay::Delay;
use stm32f3_discovery::stm32f3xx_hal::{pac, prelude::*};
use stm32f3_discovery::leds::Leds;
use stm32f3_discovery::switch_hal::{OutputSwitch, ToggleableOutputSwitch};

#[entry]
fn main()-> ! {
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
        &mut gpioe.otyper
    );
    let delay_ms = 200u16;
    loop {
        leds.ld3.toggle().ok();
        delay.delay_ms(delay_ms);
        leds.ld3.toggle().ok();
        delay.delay_ms(delay_ms);

        //explicit on/off
        leds.ld4.on().ok();
        delay.delay_ms(delay_ms);
        leds.ld4.off().ok();
        delay.delay_ms(delay_ms);

        leds.ld5.on().ok();
        delay.delay_ms(delay_ms);
        leds.ld5.off().ok();
        delay.delay_ms(delay_ms);

        leds.ld6.on().ok();
        delay.delay_ms(delay_ms);
        leds.ld6.off().ok();
        delay.delay_ms(delay_ms);
        
        leds.ld7.on().ok();
        delay.delay_ms(delay_ms);
        leds.ld7.off().ok();
        delay.delay_ms(delay_ms);
        
        leds.ld8.on().ok();
        delay.delay_ms(delay_ms);
        leds.ld8.off().ok();
        delay.delay_ms(delay_ms);
        
        leds.ld9.on().ok();
        delay.delay_ms(delay_ms);
        leds.ld9.off().ok();
        delay.delay_ms(delay_ms);

        leds.ld10.on().ok();
        delay.delay_ms(delay_ms);
        leds.ld10.off().ok();
        delay.delay_ms(delay_ms);
    }
}
