use std::any::Any;

/// This trait encapsulates being able to cast a trait object to its original concrete type
/// TODO: Add example code and maybe write derive macro because this code is always the same
pub trait AsAny {
    fn as_any(&self) -> &dyn Any;
    fn as_mut_any(&mut self) -> &mut dyn Any;
}
