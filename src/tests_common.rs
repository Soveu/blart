//! Helper function for writing tests

use std::{collections::HashSet, iter};

use crate::{header::NodeHeader, AsBytes, InsertPrefixError, InsertResult, OpaqueNodePtr};

/// Generate an iterator of bytestring keys, with increasing length up to a
/// maximum value.
///
/// This iterator will produce `max_len` number of keys. Each key has the form
/// `[0*, u8::MAX]`, meaning zero or more 0 values, followed by a single
/// `u8::MAX` value. The final `u8::MAX` value is added to ensure that no key is
/// a prefix of another key generated by this function.
///
/// # Examples
///
/// ```
/// # use blart::tests_common::generate_keys_skewed;
/// let keys = generate_keys_skewed(10).collect::<Vec<_>>();
/// assert_eq!(keys.len(), 10);
/// assert_eq!(keys[0].as_ref(), &[255]);
/// assert_eq!(keys[keys.len() - 1].as_ref(), &[0, 0, 0, 0, 0, 0, 0, 0, 0, 255]);
///
/// for k in keys {
///     println!("{:?}", k);
/// }
/// ```
///
/// The above example will print
/// ```text
/// [255]
/// [0, 255]
/// [0, 0, 255]
/// [0, 0, 0, 255]
/// [0, 0, 0, 0, 255]
/// [0, 0, 0, 0, 0, 255]
/// [0, 0, 0, 0, 0, 0, 255]
/// [0, 0, 0, 0, 0, 0, 0, 255]
/// [0, 0, 0, 0, 0, 0, 0, 0, 255]
/// [0, 0, 0, 0, 0, 0, 0, 0, 0, 255]
/// ```
///
/// # Panics
///  - Panics if `max_len` is 0.
pub fn generate_keys_skewed(max_len: usize) -> impl Iterator<Item = Box<[u8]>> {
    assert!(max_len > 0, "the fixed key length must be greater than 0");

    iter::successors(Some(vec![u8::MAX; 1].into_boxed_slice()), move |prev| {
        if prev.len() < max_len {
            let mut key = vec![u8::MIN; prev.len()];
            key.push(u8::MAX);
            Some(key.into_boxed_slice())
        } else {
            None
        }
    })
}

/// Generate an iterator of bytestring keys, all with the same length.
///
/// The `level_widths` argument specifies the number of values generated per
/// digit of the array. For example, using `[3, 2, 1]` will generate keys of
/// length 3. The generate keys will have 4 (3 + 1) unique values for the first
/// digit, 3 unique values for the second digit, and 2 unique values for the
/// last digit. In general, this iterator will produce `(level_widths[0] + 1)  *
/// (level_widths[1] + 1) * ... * (level_widths[KEY_LENGTH - 1] + 1)` keys in
/// total.
///
/// # Examples
///
/// ```
/// # use blart::tests_common::generate_key_fixed_length;
/// let keys = generate_key_fixed_length([3, 2, 1]).collect::<Vec<_>>();
/// assert_eq!(keys.len(), 24);
/// assert_eq!(keys[0].as_ref(), &[0, 0, 0]);
/// assert_eq!(keys[keys.len() / 2].as_ref(), &[170, 0, 0]);
/// assert_eq!(keys[keys.len() - 1].as_ref(), &[255, 255, 255]);
///
/// for k in keys {
///     println!("{:?}", k);
/// }
/// ```
///
/// The above example will print
/// ```text
/// [0, 0, 0]
/// [0, 0, 255]
/// [0, 128, 0]
/// [0, 128, 255]
/// [0, 255, 0]
/// [0, 255, 255]
/// [85, 0, 0]
/// [85, 0, 255]
/// [85, 128, 0]
/// [85, 128, 255]
/// [85, 255, 0]
/// [85, 255, 255]
/// [170, 0, 0]
/// [170, 0, 255]
/// [170, 128, 0]
/// [170, 128, 255]
/// [170, 255, 0]
/// [170, 255, 255]
/// [255, 0, 0]
/// [255, 0, 255]
/// [255, 128, 0]
/// [255, 128, 255]
/// [255, 255, 0]
/// [255, 255, 255]
/// ```
///
/// # Panics
///
///  - Panics if `max_len` is 0.
///  - Panics if `value_stops` is 0.
pub fn generate_key_fixed_length<const KEY_LENGTH: usize>(
    level_widths: [u8; KEY_LENGTH],
) -> impl Iterator<Item = Box<[u8]>> {
    struct FixedLengthKeys<const KEY_LENGTH: usize> {
        increments: [u8; KEY_LENGTH],
        next_value: Option<Box<[u8]>>,
    }

    impl<const KEY_LENGTH: usize> FixedLengthKeys<KEY_LENGTH> {
        pub fn new(level_widths: [u8; KEY_LENGTH]) -> Self {
            fn div_ceil(lhs: u8, rhs: u8) -> u8 {
                let d = lhs / rhs;
                let r = lhs % rhs;
                if r > 0 && rhs > 0 {
                    d + 1
                } else {
                    d
                }
            }

            assert!(
                KEY_LENGTH > 0,
                "the fixed key length must be greater than 0"
            );
            assert!(
                level_widths.iter().all(|value_stops| value_stops > &0),
                "the number of distinct values for each key digit must be greater than 0"
            );

            let increments = level_widths.map(|value_stops| div_ceil(u8::MAX, value_stops));

            FixedLengthKeys {
                increments,
                next_value: Some(vec![u8::MIN; KEY_LENGTH].into_boxed_slice()),
            }
        }
    }

    impl<const KEY_LENGTH: usize> Iterator for FixedLengthKeys<KEY_LENGTH> {
        type Item = Box<[u8]>;

        fn next(&mut self) -> Option<Self::Item> {
            let next_value = self.next_value.take()?;

            if next_value.iter().all(|digit| *digit == u8::MAX) {
                // the .take function already updated the next_value to None
                return Some(next_value);
            }

            let mut new_next_value = next_value.clone();
            for idx in (0..new_next_value.len()).rev() {
                if new_next_value[idx] == u8::MAX {
                    new_next_value[idx] = u8::MIN;
                } else {
                    new_next_value[idx] = new_next_value[idx].saturating_add(self.increments[idx]);
                    break;
                }
            }

            self.next_value = Some(new_next_value);
            Some(next_value)
        }
    }

    FixedLengthKeys::new(level_widths)
}

/// A single expansion of an existing existing that take an element at a
/// specified index and copies it multiple times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrefixExpansion {
    /// The index in an unspecified sequence that will be copied.
    pub base_index: usize,
    /// The number of copies of the original element to create.
    pub expanded_length: usize,
}

/// Generate an iterator of fixed length bytestring keys, where specific
/// portions of the key are expanded as duplicate bytes.
///
/// This is meant to simulate keys with shared prefixes in different portions of
/// the key string.
///
/// # Examples
///
/// ```
/// # use blart::tests_common::{generate_key_with_prefix, PrefixExpansion};
/// let keys = generate_key_with_prefix([2; 3], [PrefixExpansion { base_index: 0, expanded_length: 3 }]).collect::<Vec<_>>();
/// assert_eq!(keys.len(), 27);
/// assert_eq!(keys[0].as_ref(), &[0, 0, 0, 0, 0]);
/// assert_eq!(keys[(keys.len() / 2) - 2].as_ref(), &[128, 128, 128, 0, 255]);
/// assert_eq!(keys[keys.len() - 1].as_ref(), &[255, 255, 255, 255, 255]);
///
/// for k in keys {
///     println!("{:?}", k);
/// }
/// ```
///
/// The above example will print out:
/// ```text
/// [0, 0, 0, 0, 0]
/// [0, 0, 0, 0, 128]
/// [0, 0, 0, 0, 255]
/// [0, 0, 0, 128, 0]
/// [0, 0, 0, 128, 128]
/// [0, 0, 0, 128, 255]
/// [0, 0, 0, 255, 0]
/// [0, 0, 0, 255, 128]
/// [0, 0, 0, 255, 255]
/// [128, 128, 128, 0, 0]
/// [128, 128, 128, 0, 128]
/// [128, 128, 128, 0, 255]
/// [128, 128, 128, 128, 0]
/// [128, 128, 128, 128, 128]
/// [128, 128, 128, 128, 255]
/// [128, 128, 128, 255, 0]
/// [128, 128, 128, 255, 128]
/// [128, 128, 128, 255, 255]
/// [255, 255, 255, 0, 0]
/// [255, 255, 255, 0, 128]
/// [255, 255, 255, 0, 255]
/// [255, 255, 255, 128, 0]
/// [255, 255, 255, 128, 128]
/// [255, 255, 255, 128, 255]
/// [255, 255, 255, 255, 0]
/// [255, 255, 255, 255, 128]
/// [255, 255, 255, 255, 255]
/// ```
///
/// # Panics
///
///  - Panics if `base_key_len` is 0.
///  - Panics if `value_stops` is 0.
///  - Panics if any `PrefixExpansion` has `expanded_length` equal to 0.
///  - Panics if any `PrefixExpansion` has `base_index` greater than or equal to
///    `base_key_len`.
pub fn generate_key_with_prefix<const KEY_LENGTH: usize>(
    level_widths: [u8; KEY_LENGTH],
    prefix_expansions: impl AsRef<[PrefixExpansion]>,
) -> impl Iterator<Item = Box<[u8]>> {
    fn apply_expansions_to_key(
        old_key: &[u8],
        new_key_template: &[u8],
        sorted_expansions: &[PrefixExpansion],
    ) -> Box<[u8]> {
        let mut new_key: Box<[u8]> = new_key_template.into();
        let mut new_key_index = 0usize;
        let mut old_key_index = 0usize;

        for expansion in sorted_expansions {
            let before_len = expansion.base_index - old_key_index;
            new_key[new_key_index..(new_key_index + before_len)]
                .copy_from_slice(&old_key[old_key_index..expansion.base_index]);
            new_key[(new_key_index + before_len)
                ..(new_key_index + before_len + expansion.expanded_length)]
                .fill(old_key[expansion.base_index]);

            old_key_index = expansion.base_index + 1;
            new_key_index += before_len + expansion.expanded_length;
        }

        // copy over remaining bytes from the old_key
        new_key[new_key_index..].copy_from_slice(&old_key[old_key_index..]);

        new_key
    }

    let expansions = prefix_expansions.as_ref();

    assert!(
        expansions
            .iter()
            .all(|expand| { expand.base_index < KEY_LENGTH }),
        "the prefix expansion index must be less than `base_key_len`."
    );
    assert!(
        expansions
            .iter()
            .all(|expand| { expand.expanded_length > 0 }),
        "the prefix expansion length must be greater than 0."
    );
    {
        let mut uniq_indices = HashSet::new();
        assert!(
            expansions
                .iter()
                .all(|expand| uniq_indices.insert(expand.base_index)),
            "the prefix expansion index must be unique"
        );
    }

    let mut sorted_expansions = expansions.to_vec();
    sorted_expansions.sort_by(|a, b| a.base_index.cmp(&b.base_index));

    let full_key_len = expansions
        .iter()
        .map(|expand| expand.expanded_length - 1)
        .sum::<usize>()
        + KEY_LENGTH;
    let full_key_template = vec![u8::MIN; full_key_len].into_boxed_slice();

    generate_key_fixed_length(level_widths)
        .map(move |key| apply_expansions_to_key(&key, &full_key_template, &sorted_expansions))
}

#[allow(dead_code)]
pub(crate) unsafe fn insert_unchecked<'a, K, V, const NUM_PREFIX_BYTES: usize, H>(
    root: OpaqueNodePtr<K, V, NUM_PREFIX_BYTES, H>,
    key: K,
    value: V,
) -> Result<InsertResult<'a, K, V, NUM_PREFIX_BYTES, H>, InsertPrefixError>
where
    K: AsBytes + 'a,
    H: NodeHeader<NUM_PREFIX_BYTES>
{
    use crate::search_for_insert_point;

    let insert_point = unsafe { search_for_insert_point(root, &key)? };
    Ok(insert_point.apply(key, value))
}

#[allow(dead_code)]
pub(crate) fn setup_tree_from_entries<V, const NUM_PREFIX_BYTES: usize, H: NodeHeader<NUM_PREFIX_BYTES>>(
    mut entries_it: impl Iterator<Item = (Box<[u8]>, V)>,
) -> OpaqueNodePtr<Box<[u8]>, V, NUM_PREFIX_BYTES, H> {
    use crate::{LeafNode, NodePtr};

    let (first_key, first_value) = entries_it.next().unwrap();

    let mut current_root =
        NodePtr::allocate_node_ptr(LeafNode::new(first_key, first_value)).to_opaque();

    for (key, value) in entries_it {
        current_root = unsafe { insert_unchecked(current_root, key, value).unwrap().new_root };
    }

    current_root
}
