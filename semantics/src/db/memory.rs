use std::sync::Arc;

use tokio::sync::RwLock;

struct State {

}

pub struct MemoryDb {
    state: Arc<RwLock<State>>,
}

impl MemoryDb {
    pub fn new() -> Self {
        Self{
            state: Arc::new(RwLock::new(State{

            }))
        }
    }
}


