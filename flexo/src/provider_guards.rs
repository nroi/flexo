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

    pub fn get_provider_guard<F, O>(&self, mirror_score: F) -> (ProviderGuard<P>, usize)
        where F: Fn(&P) -> ProviderChoice<O>, O: Ord + Copy
    {
        let _lock = self.mutex.lock().unwrap();
        let guards_with_scores = self.guards.iter()
            .filter_map(|g| {
                match mirror_score(&g.guarded_provider) {
                    ProviderChoice::Include(score) => Some((g, score)),
                    ProviderChoice::Exclude => None,
                }
            }).collect::<Vec<(&ProviderGuard<P>, O)>>();
        // Avoid multiple parallel downloads from the same remote mirror: We prefer to download from different mirrors
        // instead, to increase the chance that the client's bandwidth is saturated.
        let (guard, _) = guards_with_scores.iter().min_by_key(|(g, score)| {
            (g.num_current_usages(), *score)
        }).unwrap();
        debug!("Selected {:?}, number of usages: {} [{:?}]",
                 &guard.guarded_provider, guard.num_current_usages(), std::thread::current().id());
        let guard = ProviderGuard {
            guarded_provider: Arc::clone(&guard.guarded_provider)
        };
        (guard, guards_with_scores.len())
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