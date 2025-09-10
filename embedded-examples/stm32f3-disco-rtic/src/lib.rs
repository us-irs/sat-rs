#![no_main]
#![no_std]

use cortex_m_semihosting::debug;

use defmt_rtt as _; // global logger

use panic_probe as _;

use embassy_stm32::gpio::Output;

#[derive(defmt::Format, PartialEq, Eq, Clone, Copy)]
pub enum Direction {
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}

impl Direction {
    pub fn switch_to_next(&mut self) -> (Self, Self) {
        let curr = *self;
        *self = match self {
            Direction::North => Direction::NorthEast,
            Direction::NorthEast => Direction::East,
            Direction::East => Direction::SouthEast,
            Direction::SouthEast => Direction::South,
            Direction::South => Direction::SouthWest,
            Direction::SouthWest => Direction::West,
            Direction::West => Direction::NorthWest,
            Direction::NorthWest => Direction::North,
        };
        (curr, *self)
    }
}

pub struct Leds {
    pub north: Output<'static>,
    pub north_east: Output<'static>,
    pub east: Output<'static>,
    pub south_east: Output<'static>,
    pub south: Output<'static>,
    pub south_west: Output<'static>,
    pub west: Output<'static>,
    pub north_west: Output<'static>,
}

impl Leds {
    pub fn blink_next(&mut self, current_dir: &mut Direction) {
        let (prev, curr) = current_dir.switch_to_next();
        self.set_dir_low(prev);
        self.set_dir_high(curr);
    }

    pub fn set_dir(&mut self, dir: Direction, level: embassy_stm32::gpio::Level) {
        match dir {
            Direction::North => self.north.set_level(level),
            Direction::NorthEast => self.north_east.set_level(level),
            Direction::East => self.east.set_level(level),
            Direction::SouthEast => self.south_east.set_level(level),
            Direction::South => self.south.set_level(level),
            Direction::SouthWest => self.south_west.set_level(level),
            Direction::West => self.west.set_level(level),
            Direction::NorthWest => self.north_west.set_level(level),
        }
    }

    pub fn set_dir_low(&mut self, dir: Direction) {
        self.set_dir(dir, embassy_stm32::gpio::Level::Low);
    }

    pub fn set_dir_high(&mut self, dir: Direction) {
        self.set_dir(dir, embassy_stm32::gpio::Level::High);
    }
}

pub struct LedPinSet {
    pub pin_n: embassy_stm32::Peri<'static, embassy_stm32::peripherals::PE8>,
    pub pin_ne: embassy_stm32::Peri<'static, embassy_stm32::peripherals::PE9>,
    pub pin_e: embassy_stm32::Peri<'static, embassy_stm32::peripherals::PE10>,
    pub pin_se: embassy_stm32::Peri<'static, embassy_stm32::peripherals::PE11>,
    pub pin_s: embassy_stm32::Peri<'static, embassy_stm32::peripherals::PE12>,
    pub pin_sw: embassy_stm32::Peri<'static, embassy_stm32::peripherals::PE13>,
    pub pin_w: embassy_stm32::Peri<'static, embassy_stm32::peripherals::PE14>,
    pub pin_nw: embassy_stm32::Peri<'static, embassy_stm32::peripherals::PE15>,
}

impl Leds {
    pub fn new(pin_set: LedPinSet) -> Self {
        let led_n = Output::new(
            pin_set.pin_n,
            embassy_stm32::gpio::Level::Low,
            embassy_stm32::gpio::Speed::Medium,
        );
        let led_ne = Output::new(
            pin_set.pin_ne,
            embassy_stm32::gpio::Level::Low,
            embassy_stm32::gpio::Speed::Medium,
        );
        let led_e = Output::new(
            pin_set.pin_e,
            embassy_stm32::gpio::Level::Low,
            embassy_stm32::gpio::Speed::Medium,
        );
        let led_se = Output::new(
            pin_set.pin_se,
            embassy_stm32::gpio::Level::Low,
            embassy_stm32::gpio::Speed::Medium,
        );
        let led_s = Output::new(
            pin_set.pin_s,
            embassy_stm32::gpio::Level::Low,
            embassy_stm32::gpio::Speed::Medium,
        );
        let led_sw = Output::new(
            pin_set.pin_sw,
            embassy_stm32::gpio::Level::Low,
            embassy_stm32::gpio::Speed::Medium,
        );
        let led_w = Output::new(
            pin_set.pin_w,
            embassy_stm32::gpio::Level::Low,
            embassy_stm32::gpio::Speed::Medium,
        );
        let led_nw = Output::new(
            pin_set.pin_nw,
            embassy_stm32::gpio::Level::Low,
            embassy_stm32::gpio::Speed::Medium,
        );
        Self {
            north: led_n,
            north_east: led_ne,
            east: led_e,
            south_east: led_se,
            south: led_s,
            south_west: led_sw,
            west: led_w,
            north_west: led_nw,
        }
    }
}

// same panicking *behavior* as `panic-probe` but doesn't print a panic message
// this prevents the panic message being printed *twice* when `defmt::panic` is invoked
#[defmt::panic_handler]
fn panic() -> ! {
    cortex_m::asm::udf()
}

/// Terminates the application and makes a semihosting-capable debug tool exit
/// with status code 0.
pub fn exit() -> ! {
    loop {
        debug::exit(debug::EXIT_SUCCESS);
    }
}

/// Hardfault handler.
///
/// Terminates the application and makes a semihosting-capable debug tool exit
/// with an error. This seems better than the default, which is to spin in a
/// loop.
#[cortex_m_rt::exception]
unsafe fn HardFault(_frame: &cortex_m_rt::ExceptionFrame) -> ! {
    loop {
        debug::exit(debug::EXIT_FAILURE);
    }
}

// defmt-test 0.3.0 has the limitation that this `#[tests]` attribute can only be used
// once within a crate. the module can be in any file but there can only be at most
// one `#[tests]` module in this library crate
#[cfg(test)]
#[defmt_test::tests]
mod unit_tests {
    use defmt::assert;

    #[test]
    fn it_works() {
        assert!(true)
    }
}
