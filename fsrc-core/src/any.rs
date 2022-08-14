use std::any::Any;

pub trait AsAny {
    // I am not 100 % sure this is the best idea, but it allows downcasting the trait object
    // to its original concrete type.
    fn as_any(&self) -> &dyn Any;
    fn as_mut_any(&mut self) -> &mut dyn Any;
}
