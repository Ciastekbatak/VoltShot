use std::collections::HashMap;
use std::hash::Hash;
use std::num::NonZeroUsize;
use std::sync::Arc;

/// Default maximum number of decoded screenshot thumbnails kept in memory.
pub const DEFAULT_THUMB_CACHE_CAPACITY: usize = 128;

struct Node<K, V> {
    key: Arc<K>,
    value: V,
    prev: Option<usize>,
    next: Option<usize>,
}

/// Bounded O(1) least-recently-used cache for keeping thumbnail textures and entries capped in RAM.
pub struct LruCache<K, V> {
    capacity: NonZeroUsize,
    map: HashMap<Arc<K>, usize>,
    nodes: Vec<Option<Node<K, V>>>,
    free_slots: Vec<usize>,
    head: Option<usize>,
    tail: Option<usize>,
}

impl<K, V> LruCache<K, V>
where
    K: Eq + Hash,
{
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self {
            capacity,
            map: HashMap::with_capacity(capacity.get()),
            nodes: Vec::with_capacity(capacity.get()),
            free_slots: Vec::new(),
            head: None,
            tail: None,
        }
    }

    pub fn capacity(&self) -> NonZeroUsize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn contains_key(&self, key: &K) -> bool {
        self.map.contains_key(key)
    }

    pub fn peek(&self, key: &K) -> Option<&V> {
        let &idx = self.map.get(key)?;
        self.nodes[idx].as_ref().map(|node| &node.value)
    }

    /// Looks up `key`, promoting it to most-recently-used if present.
    pub fn get(&mut self, key: &K) -> Option<&V> {
        let &idx = self.map.get(key)?;
        self.move_to_front(idx);
        self.nodes[idx].as_ref().map(|node| &node.value)
    }

    /// Inserts or updates `(key, value)`, marking it most-recently-used.
    ///
    /// Overwriting an existing key updates its value in place without growing the cache and
    /// returns `None`. Inserting a new key when at capacity evicts and returns the least-recently-used
    /// `(key, value)` pair.
    pub fn insert(&mut self, key: K, value: V) -> Option<(K, V)> {
        if let Some(&idx) = self.map.get(&key) {
            if let Some(node) = self.nodes[idx].as_mut() {
                node.value = value;
            }
            self.move_to_front(idx);
            return None;
        }

        let evicted = if self.map.len() >= self.capacity.get() {
            self.pop_tail()
        } else {
            None
        };

        let shared_key = Arc::new(key);
        let node = Node {
            key: Arc::clone(&shared_key),
            value,
            prev: None,
            next: None,
        };
        let idx = if let Some(slot) = self.free_slots.pop() {
            self.nodes[slot] = Some(node);
            slot
        } else {
            let slot = self.nodes.len();
            self.nodes.push(Some(node));
            slot
        };

        self.map.insert(shared_key, idx);
        self.push_front(idx);
        evicted
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let idx = self.map.remove(key)?;
        self.unlink(idx);
        let node = self.nodes[idx].take()?;
        self.free_slots.push(idx);
        Some(node.value)
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.nodes.clear();
        self.free_slots.clear();
        self.head = None;
        self.tail = None;
    }

    fn move_to_front(&mut self, idx: usize) {
        if self.head == Some(idx) {
            return;
        }
        self.unlink(idx);
        self.push_front(idx);
    }

    fn push_front(&mut self, idx: usize) {
        let old_head = self.head;
        if let Some(node) = self.nodes[idx].as_mut() {
            node.prev = None;
            node.next = old_head;
        }
        if let Some(h) = old_head {
            if let Some(head_node) = self.nodes[h].as_mut() {
                head_node.prev = Some(idx);
            }
        } else {
            self.tail = Some(idx);
        }
        self.head = Some(idx);
    }

    fn unlink(&mut self, idx: usize) {
        let (prev, next) = match self.nodes[idx].as_ref() {
            Some(node) => (node.prev, node.next),
            None => return,
        };

        if let Some(p) = prev {
            if let Some(prev_node) = self.nodes[p].as_mut() {
                prev_node.next = next;
            }
        } else {
            self.head = next;
        }

        if let Some(n) = next {
            if let Some(next_node) = self.nodes[n].as_mut() {
                next_node.prev = prev;
            }
        } else {
            self.tail = prev;
        }

        if let Some(node) = self.nodes[idx].as_mut() {
            node.prev = None;
            node.next = None;
        }
    }

    fn pop_tail(&mut self) -> Option<(K, V)> {
        let tail_idx = self.tail?;
        self.unlink(tail_idx);
        let node = self.nodes[tail_idx].take()?;
        self.free_slots.push(tail_idx);
        self.map.remove(node.key.as_ref());
        let owned_key = Arc::into_inner(node.key)?;
        Some((owned_key, node.value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("non-zero capacity")
    }

    #[test]
    fn evicts_truly_least_recently_used_entry_after_get_refresh() {
        let mut cache = LruCache::new(cap(3));
        assert!(cache.is_empty());
        assert_eq!(cache.insert("a", 1), None);
        assert_eq!(cache.insert("b", 2), None);
        assert_eq!(cache.insert("c", 3), None);
        assert_eq!(cache.len(), 3);

        assert_eq!(cache.get(&"a"), Some(&1));

        assert_eq!(cache.insert("d", 4), Some(("b", 2)));
        assert_eq!(cache.len(), 3);
        assert!(!cache.contains_key(&"b"));
        assert_eq!(cache.peek(&"a"), Some(&1));
        assert_eq!(cache.peek(&"c"), Some(&3));
        assert_eq!(cache.peek(&"d"), Some(&4));

        assert_eq!(cache.insert("e", 5), Some(("c", 3)));
        assert!(!cache.contains_key(&"c"));
        assert_eq!(cache.get(&"a"), Some(&1));
        assert_eq!(cache.insert("f", 6), Some(("d", 4)));
    }

    #[test]
    fn overwrite_updates_value_and_recency_without_growing_len() {
        let mut cache = LruCache::new(cap(2));
        assert_eq!(cache.insert("first", 10), None);
        assert_eq!(cache.insert("second", 20), None);
        assert_eq!(cache.len(), 2);

        assert_eq!(cache.insert("first", 100), None);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.peek(&"first"), Some(&100));

        assert_eq!(cache.insert("third", 30), Some(("second", 20)));
        assert_eq!(cache.get(&"first"), Some(&100));
        assert_eq!(cache.get(&"third"), Some(&30));
    }

    #[test]
    fn removal_handles_head_middle_tail_and_clear() {
        let mut cache = LruCache::new(cap(3));
        cache.insert(1, "one");
        cache.insert(2, "two");
        cache.insert(3, "three");

        assert_eq!(cache.remove(&2), Some("two"));
        assert_eq!(cache.remove(&2), None);
        assert_eq!(cache.len(), 2);

        assert_eq!(cache.remove(&3), Some("three"));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&1), Some(&"one"));

        cache.insert(4, "four");
        cache.insert(5, "five");
        assert_eq!(cache.len(), 3);
        assert_eq!(cache.insert(6, "six"), Some((1, "one")));

        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.get(&4), None);

        assert_eq!(cache.insert(7, "seven"), None);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&7), Some(&"seven"));
    }

    #[test]
    fn capacity_one_cache_evicts_and_removes_cleanly() {
        let mut cache = LruCache::new(cap(1));
        assert_eq!(cache.capacity(), cap(1));
        assert_eq!(cache.insert("k1", 1), None);
        assert_eq!(cache.insert("k1", 2), None);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.insert("k2", 3), Some(("k1", 2)));
        assert_eq!(cache.remove(&"k2"), Some(3));
        assert!(cache.is_empty());
    }
}
