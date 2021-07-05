use std::sync::{Arc, Mutex};
use std::fmt::Debug;

pub struct ProviderGuards<P> where P: Debug {
    guards: Vec<ProviderGuard<P>>,
    mutex: Mutex<()>,
}

impl <P> ProviderGuards<P> where P: Debug {
    pub fn new(items: Vec<P>) -> Self {
        let guards = items.into_iter()
            .map(ProviderGuard::new)
            .collect();
        Self {
            guards,
            mutex: Mutex::new(()),
        }
    }

    pub fn get_provider_guard<F, O>(&self, f: F) -> (ProviderGuard<P>, usize)
        where F: Fn(&P) -> ProviderChoice<O>, O: Ord + Copy
    {
        let _lock = self.mutex.lock().unwrap();
        let intermediate = self.guards.iter()
            .filter_map(|g| {
                match f(&g.guarded_provider) {
                    ProviderChoice::Include(o) => Some((g, o)),
                    ProviderChoice::Exclude => None,
                }
            }).collect::<Vec<(&ProviderGuard<P>, O)>>();
        let (guard, _) = intermediate.iter().min_by_key(|(g, o)| {
            (g.num_current_usages(), *o)
        }).unwrap();
        debug!("Selected {:?}, number of usages: {} [{:?}]",
                 &guard.guarded_provider, guard.num_current_usages(), std::thread::current().id());
        let guard = ProviderGuard {
            guarded_provider: Arc::clone(&guard.guarded_provider)
        };
        (guard, intermediate.len())
    }
}

pub enum ProviderChoice<O> {
    Include(O),
    Exclude,
}

/// Wraps a provider to keep track of how often it is currently in use.
#[derive(Debug)]
pub struct ProviderGuard<P> where P: Debug {
    pub guarded_provider: Arc<P>,
}

impl <P> ProviderGuard<P> where P: Debug {
    pub fn new(provider: P) -> Self {
        Self {
            guarded_provider: Arc::new(provider),
        }
    }

    pub fn num_current_usages(&self) -> usize {
        Arc::strong_count(&self.guarded_provider)
    }
}