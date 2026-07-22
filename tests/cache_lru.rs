/// LRU cache eviction tests for the GlesTexture cache used by the compositor.
///
/// The production texture cache in `src/backend/mod.rs` is a
/// `lru::LruCache<ObjectId, TextureBuffer<GlesTexture>>` with capacity 256.
/// These tests validate the underlying LRU data structure directly
/// (no GL context required).
use lru::LruCache;
use std::num::NonZeroUsize;

/// Insert entries into the cache and verify they are retrievable.
#[test]
fn test_insert_and_retrieve() {
    let mut cache: LruCache<u32, String> =
        LruCache::new(NonZeroUsize::new(256).unwrap());

    cache.put(1, "one".to_string());
    cache.put(2, "two".to_string());
    cache.put(3, "three".to_string());

    assert_eq!(cache.get(&1), Some(&"one".to_string()));
    assert_eq!(cache.get(&2), Some(&"two".to_string()));
    assert_eq!(cache.get(&3), Some(&"three".to_string()));
    assert_eq!(cache.get(&4), None);
}

/// Exceed the cache capacity and verify the oldest entries are dropped.
#[test]
fn test_eviction_on_capacity_exceeded() {
    let mut cache: LruCache<u32, u32> =
        LruCache::new(NonZeroUsize::new(4).unwrap());

    // Fill the cache to capacity.
    cache.put(1, 10);
    cache.put(2, 20);
    cache.put(3, 30);
    cache.put(4, 40);

    // Inserting a fifth entry should evict the oldest (1).
    cache.put(5, 50);

    assert_eq!(
        cache.get(&1),
        None,
        "oldest entry (1) should be evicted"
    );
    assert_eq!(cache.get(&2), Some(&20));
    assert_eq!(cache.get(&3), Some(&30));
    assert_eq!(cache.get(&4), Some(&40));
    assert_eq!(cache.get(&5), Some(&50));
}

/// Verify the cache handles duplicate keys by overwriting the value.
#[test]
fn test_duplicate_keys_overwrite() {
    let mut cache: LruCache<&str, i32> =
        LruCache::new(NonZeroUsize::new(256).unwrap());

    cache.put("a", 1);
    cache.put("b", 2);
    cache.put("a", 99); // overwrite

    assert_eq!(
        cache.get(&"a"),
        Some(&99),
        "value should be updated on duplicate key"
    );
    assert_eq!(cache.get(&"b"), Some(&2));
    assert_eq!(cache.len(), 2, "cache should still have 2 entries");
}

/// Verify that accessing an entry promotes it, so a different entry is evicted.
#[test]
fn test_lru_promotion_on_access() {
    let mut cache: LruCache<char, u32> =
        LruCache::new(NonZeroUsize::new(3).unwrap());

    cache.put('a', 1);
    cache.put('b', 2);
    cache.put('c', 3);

    // Access 'a' to promote it to most-recently-used.
    let _ = cache.get(&'a');

    // Insert 'd' — should evict 'b' (the LRU, not 'a' which was just touched).
    cache.put('d', 4);

    assert_eq!(
        cache.get(&'a'),
        Some(&1),
        "recently accessed entry ('a') should be preserved"
    );
    assert_eq!(
        cache.get(&'b'),
        None,
        "LRU entry ('b') should be evicted after 'a' was promoted"
    );
    assert_eq!(cache.get(&'c'), Some(&3));
    assert_eq!(cache.get(&'d'), Some(&4));
}

/// Verify the cache behaves correctly at the production capacity (256 entries).
#[test]
fn test_production_capacity() {
    let mut cache: LruCache<u32, u32> =
        LruCache::new(NonZeroUsize::new(256).unwrap());

    for i in 0..256 {
        cache.put(i, i * 10);
    }

    // All 256 entries should be present.
    assert_eq!(cache.len(), 256);

    // Touch entry 0 so it's most-recently-used.
    let _ = cache.get(&0);
    assert_eq!(cache.get(&0), Some(&0));

    // Insert one more — should evict entry 1 (the LRU after touching 0).
    cache.put(256, 2560);
    assert_eq!(cache.len(), 256);
    assert_eq!(
        cache.get(&0),
        Some(&0),
        "recently touched entry (0) preserved at capacity"
    );
    assert_eq!(
        cache.get(&1),
        None,
        "LRU entry (1) should be evicted at capacity"
    );
    assert_eq!(cache.get(&256), Some(&2560));
}
