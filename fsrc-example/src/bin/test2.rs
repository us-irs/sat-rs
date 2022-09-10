// use std::sync::{Arc, Mutex};
//
// trait ProvidesFoo {
//     fn magic(&mut self);
// }
//
// struct Foo {
//     magic_value: u32,
// }
//
// impl Default for Foo {
//     fn default() -> Self {
//         Self { magic_value: 42 }
//     }
// }
//
// impl ProvidesFoo for Foo {
//     fn magic(&mut self) {
//         println!("ProvidesFoo magic {}", self.magic_value);
//     }
// }
//
// type SharedFooConcrete = Arc<Mutex<Box<Foo>>>;
// type SharedFooTraitObj = Arc<Mutex<Box<dyn ProvidesFoo + Send + Sync>>>;
//
// #[allow(dead_code)]
// struct FooProvider {
//     foo_as_trait_obj: SharedFooTraitObj,
// }
//
// impl FooProvider {
//     fn magic_and_then_some(&mut self) {
//         let mut fooguard = self.foo_as_trait_obj.lock().unwrap();
//         fooguard.magic();
//         println!("Additional magic");
//     }
// }
//
// #[allow(dead_code)]
// fn uses_shared_foo_boxed_trait_obj(foo: SharedFooTraitObj) {
//     let mut foo_provider = FooProvider {
//         foo_as_trait_obj: foo,
//     };
//     foo_provider.magic_and_then_some();
// }
// fn uses_shared_foo_concrete(foo: SharedFooConcrete) {
//     let mut fooguard = foo.lock().unwrap();
//     fooguard.magic();
// }
//
fn main() {
    //     let shared_foo = Arc::new(Mutex::new(Box::new(Foo::default())));
    //     uses_shared_foo_concrete(shared_foo.clone());
    //     // uses_shared_foo_boxed_trait_obj(shared_foo);
}
