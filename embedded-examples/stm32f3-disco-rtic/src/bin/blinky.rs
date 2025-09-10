#![no_main]
#![no_std]

use panic_probe as _;
use rtic::app;

#[app(device = embassy_stm32)]
mod app {
    use rtic_monotonics::fugit::ExtU32;
    use rtic_monotonics::Monotonic as _;
    use satrs_stm32f3_disco_rtic::{Direction, LedPinSet, Leds};

    rtic_monotonics::systick_monotonic!(Mono, 1000);

    #[shared]
    struct Shared {}

    #[local]
    struct Local {
        leds: Leds,
        current_dir: Direction,
    }

    #[init]
    fn init(cx: init::Context) -> (Shared, Local) {
        let p = embassy_stm32::init(Default::default());

        defmt::info!("Starting sat-rs demo application for the STM32F3-Discovery using RTICv2");

        let led_pin_set = LedPinSet {
            pin_n: p.PE8,
            pin_ne: p.PE9,
            pin_e: p.PE10,
            pin_se: p.PE11,
            pin_s: p.PE12,
            pin_sw: p.PE13,
            pin_w: p.PE14,
            pin_nw: p.PE15,
        };
        let leds = Leds::new(led_pin_set);

        // Initialize the systick interrupt & obtain the token to prove that we did
        Mono::start(cx.core.SYST, 8_000_000);
        blinky::spawn().expect("failed to spawn blinky task");
        (
            Shared {},
            Local {
                leds,
                current_dir: Direction::North,
            },
        )
    }

    #[task(local = [leds, current_dir])]
    async fn blinky(cx: blinky::Context) {
        loop {
            cx.local.leds.blink_next(cx.local.current_dir);
            Mono::delay(200.millis()).await;
        }
    }
}
