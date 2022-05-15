use std::any::Any;
use std::collections::HashMap;
use std::error::Error;
// use thiserror::Error;

pub struct Event {
    event_id: u32,
}

#[derive(PartialEq, Eq, Hash, Copy, Clone)]
pub struct ObjectId {
    id: u32,
    name: &'static str,
}

pub trait SystemObject {
    fn as_any(&self) -> &dyn Any;
    fn get_object_id(&self) -> &ObjectId;
    fn initialize(&mut self) -> Result<(), Box<dyn Error>>;
}

pub trait ManagedSystemObject: SystemObject + Any {}

pub struct ObjectManager {
    obj_map: HashMap<ObjectId, Box<dyn ManagedSystemObject>>,
}

impl ObjectManager {
    pub fn new() -> ObjectManager {
        ObjectManager {
            obj_map: HashMap::new(),
        }
    }
    pub fn insert(&mut self, sys_obj: Box<dyn ManagedSystemObject>) -> bool {
        let obj_id = sys_obj.get_object_id();
        if self.obj_map.contains_key(&obj_id) {
            return false;
        }
        self.obj_map.insert(obj_id.clone(), sys_obj).is_none()
    }

    /// Initializes all System Objects in the hash map and returns the number of successful
    /// initializations
    pub fn initialize(&mut self) -> Result<u32, Box<dyn Error>> {
        let mut init_success = 0;
        for val in self.obj_map.values_mut() {
            if val.initialize().is_ok() {
                init_success += 1
            }
        }
        Ok(init_success)
    }

    pub fn get<T: Any>(&self, key: &ObjectId) -> Option<&T> {
        self.obj_map
            .get(key)
            .and_then(|o| o.as_ref().as_any().downcast_ref::<T>())
    }
}

#[cfg(test)]
mod tests {
    use crate::{ManagedSystemObject, ObjectId, ObjectManager, SystemObject};
    use std::any::Any;
    use std::error::Error;

    struct ExampleSysObj {
        id: ObjectId,
        dummy: u32,
        was_initialized: bool,
    }

    impl ExampleSysObj {
        fn new(id: ObjectId, dummy: u32) -> ExampleSysObj {
            ExampleSysObj {
                id,
                dummy,
                was_initialized: false,
            }
        }
    }

    impl SystemObject for ExampleSysObj {
        fn as_any(&self) -> &dyn Any {
            self
        }

        fn get_object_id(&self) -> &ObjectId {
            &self.id
        }

        fn initialize(&mut self) -> Result<(), Box<dyn Error>> {
            self.was_initialized = true;
            Ok(())
        }
    }

    impl ManagedSystemObject for ExampleSysObj {}

    struct OtherExampleObject {
        id: ObjectId,
        string: String,
        was_initialized: bool,
    }

    impl SystemObject for OtherExampleObject {
        fn as_any(&self) -> &dyn Any {
            self
        }

        fn get_object_id(&self) -> &ObjectId {
            &self.id
        }

        fn initialize(&mut self) -> Result<(), Box<dyn Error>> {
            self.was_initialized = true;
            Ok(())
        }
    }

    impl ManagedSystemObject for OtherExampleObject {}

    #[test]
    fn test_obj_manager_simple() {
        let mut obj_manager = ObjectManager::new();
        let expl_obj_id = ObjectId {
            id: 0,
            name: "Example 0",
        };
        let example_obj = ExampleSysObj::new(expl_obj_id, 42);
        assert_eq!(obj_manager.insert(Box::new(example_obj)), true);
        let res = obj_manager.initialize();
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), 1);
        let obj_back_casted: Option<&ExampleSysObj> = obj_manager.get(&expl_obj_id);
        assert!(obj_back_casted.is_some());
        let expl_obj_back_casted = obj_back_casted.unwrap();
        assert_eq!(expl_obj_back_casted.dummy, 42);
        assert_eq!(expl_obj_back_casted.was_initialized, true);

        let second_obj_id = ObjectId {
            id: 12,
            name: "Example 1",
        };
        let second_example_obj = OtherExampleObject {
            id: second_obj_id,
            string: String::from("Hello Test"),
            was_initialized: false,
        };

        assert_eq!(obj_manager.insert(Box::new(second_example_obj)), true);
        let res = obj_manager.initialize();
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), 2);
        let obj_back_casted: Option<&OtherExampleObject> = obj_manager.get(&second_obj_id);
        assert!(obj_back_casted.is_some());
        let expl_obj_back_casted = obj_back_casted.unwrap();
        assert_eq!(expl_obj_back_casted.string, String::from("Hello Test"));
        assert_eq!(expl_obj_back_casted.was_initialized, true);
    }
}
