use std::collections::HashMap;

/// A simple key-value store with typed entries.
#[derive(Debug, Clone)]
pub struct Store<K, V> {
    data: HashMap<K, V>,
    capacity: usize,
}

impl<K: Eq + std::hash::Hash, V> Store<K, V> {
    pub fn new(capacity: usize) -> Self {
        Store {
            data: HashMap::new(),
            capacity,
        }
    }

    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        if self.data.len() >= self.capacity {
            return None;
        }
        self.data.insert(key, value)
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.data.get(key)
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

#[derive(Debug, PartialEq)]
enum Status {
    Active,
    Inactive,
    Pending(String),
}

fn process(status: &Status) -> &'static str {
    match status {
        Status::Active => "running",
        Status::Inactive => "stopped",
        Status::Pending(_) => "waiting",
    }
}

fn fibonacci(n: u64) -> u64 {
    match n {
        0 => 0,
        1 => 1,
        _ => fibonacci(n - 1) + fibonacci(n - 2),
    }
}

fn main() {
    let mut store: Store<String, i32> = Store::new(10);
    store.insert("alpha".to_string(), 1);
    store.insert("beta".to_string(), 2);

    if let Some(val) = store.get(&"alpha".to_string()) {
        println!("alpha = {val}");
    }

    let status = Status::Pending("upload".to_string());
    println!("status: {}", process(&status));

    let fibs: Vec<u64> = (0..10).map(fibonacci).collect();
    println!("{fibs:?}");
}
