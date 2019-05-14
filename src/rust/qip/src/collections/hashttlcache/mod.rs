// todo: implement `HashMap` forwarding as needed.

#[cfg(test)]
mod tests;

use std::cmp::Ordering;
use std::collections::hash_map::Entry as HashMapEntry;
use std::collections::BinaryHeap;
use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

#[derive(PartialEq, Eq, Clone)]
struct Expiry(Instant);

impl Expiry {
    pub fn has_expired(&self, now: Instant) -> bool {
        now >= self.0
    }
}

impl Ord for Expiry {
    fn cmp(&self, other: &Expiry) -> Ordering {
        // `BinaryHeap` is a max-heap, so we need to reverse the order of comparisons in order to get `peek()` and `pop()` to return the smallest time.
        match self.0.cmp(&other.0) {
            Ordering::Equal => Ordering::Equal,
            Ordering::Less => Ordering::Greater,
            Ordering::Greater => Ordering::Less,
        }
    }
}

impl PartialOrd for Expiry {
    fn partial_cmp(&self, other: &Expiry) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

struct Record<V> {
    value: V,
    expiry: Option<Expiry>,
}

#[derive(PartialEq, Eq, Clone)]
struct Tombstone<K>
where
    K: Eq,
{
    key: K,
    expiry: Expiry,
}

impl<K> Ord for Tombstone<K>
where
    K: Eq,
{
    fn cmp(&self, other: &Tombstone<K>) -> Ordering {
        self.expiry.cmp(&other.expiry)
    }
}

impl<K> PartialOrd for Tombstone<K>
where
    K: Eq,
{
    fn partial_cmp(&self, other: &Tombstone<K>) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// todo: `HashMap<>` has an `S` parameter that i'd like to include but causes problems with the inference engine. the workaround is to leave it out but what am i doing wrong?

pub struct HashTtlCache<K, V>
where
    K: Eq + Hash,
{
    map: HashMap<K, Record<V>>,
    graveyard: BinaryHeap<Tombstone<K>>,
    default_ttl: Option<Duration>,
    now: Instant,
}

impl<K, V> HashTtlCache<K, V>
where
    K: Eq + Hash + Copy,
    V: Copy,
{
    pub fn new(
        default_ttl: Option<Duration>,
        now: Instant,
    ) -> HashTtlCache<K, V> {
        if let Some(ttl) = default_ttl {
            assert!(ttl > Duration::new(0, 0));
        }

        HashTtlCache {
            map: HashMap::new(),
            graveyard: BinaryHeap::new(),
            default_ttl,
            now,
        }
    }

    pub fn insert_with_ttl(
        &mut self,
        key: K,
        value: V,
        ttl: Option<Duration>,
    ) -> Option<V> {
        if let Some(ttl) = ttl {
            assert!(ttl > Duration::new(0, 0));
        }

        let expiry = ttl.map(|dt| Expiry(self.now + dt));

        let old_value = match self.map.entry(key) {
            HashMapEntry::Occupied(mut e) => {
                let mut record = e.get_mut();
                let old_value = if let Some(ref expiry) = record.expiry {
                    if expiry.has_expired(self.now) {
                        None
                    } else {
                        Some(record.value)
                    }
                } else {
                    Some(record.value)
                };

                record.value = value;
                record.expiry = expiry.clone();

                old_value
            }
            HashMapEntry::Vacant(e) => {
                e.insert(Record {
                    value,
                    expiry: expiry.clone(),
                });

                None
            }
        };

        if let Some(expiry) = expiry {
            let expiry = Tombstone { key, expiry };

            self.graveyard.push(expiry);
        }

        old_value
    }

    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        self.insert_with_ttl(key, value, self.default_ttl)
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        if let Some(ref record) = self.map.remove(key) {
            if let Some(ref expiry) = record.expiry {
                if !expiry.has_expired(self.now) {
                    return Some(record.value);
                }
            }
        }

        None
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.map.get(key).map(|r| &r.value)
    }

    pub fn try_evict(&mut self, now: Instant) -> HashMap<K, V> {
        assert!(now > self.now);
        self.now = now;
        let mut evicted = HashMap::new();

        loop {
            match self.try_evict_once() {
                Some((key, value)) => {
                    assert!(evicted.insert(key, value).is_none());
                }
                None => return evicted,
            }
        }
    }

    fn try_evict_once(&mut self) -> Option<(K, V)> {
        loop {
            let (key, graveyard_expiry) = match self.graveyard.peek() {
                Some(e) => ((*e).key, (*e).expiry.clone()),
                None =>
                // the graveyard is empty, so we cannot evict anything.
                {
                    return None
                }
            };

            // the next tombstone has time from the future; nothing to evict.
            if !graveyard_expiry.has_expired(self.now) {
                return None;
            }

            assert!(self.graveyard.pop().is_some());
            match self.map.entry(key) {
                HashMapEntry::Occupied(e) => {
                    let (record_expiry, value) = {
                        let record = e.get();
                        let expiry = record.expiry.as_ref().unwrap();
                        (expiry, record.value)
                    };

                    if &graveyard_expiry == record_expiry {
                        // the entry's expiry matches our tombstone; time to evict.
                        e.remove_entry();
                        return Some((key, value));
                    } else {
                        // the entry hasn't expired yet; keep looking.
                        assert!(!record_expiry.has_expired(self.now));
                        continue;
                    }
                }
                HashMapEntry::Vacant(_) => continue,
            }
        }
    }
}
