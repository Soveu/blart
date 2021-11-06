//! Trie node representation

use std::{
    cmp,
    error::Error,
    fmt,
    marker::PhantomData,
    mem::{ManuallyDrop, MaybeUninit},
    ptr::{self, NonNull},
};

/// The number of bytes stored for path compression in the node header.
pub const NUM_PREFIX_BYTES: usize = 8;

/// The representation of inner nodes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NodeType {
    /// Node that references between 2 and 4 children
    Node4 = 0b000,
    /// Node that references between 5 and 16 children
    Node16, // 0b001
    /// Node that references between 17 and 49 children
    Node48, // 0b010
    /// Node that references between 49 and 256 children
    Node256, // 0b011
    /// Node that contains a single value
    Leaf, // 0b100
}

impl NodeType {
    /// The upper bound on the number of child nodes that this
    /// NodeType can have.
    pub const fn upper_capacity(self) -> usize {
        match self {
            NodeType::Node4 => 4,
            NodeType::Node16 => 16,
            NodeType::Node48 => 48,
            NodeType::Node256 => 256,
            NodeType::Leaf => 0,
        }
    }
}

/// The common header for all inner nodes
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Header {
    /// The size (in bytes) of the prefix stored in the header
    // size 4, alignment 4
    pub prefix_size: u32,
    /// Number of children this internal node points to
    // size 2, alignment 2
    num_children: u16,
    /// The type of representation for this current node
    // size 1, alignment 1
    pub node_type: NodeType,
    /// The key prefix for this node.
    ///
    /// Only the first `prefix_size` bytes are guaranteed to be initialized.
    // size NUM_PREFIX_BYTES, alignment 1
    pub prefix: [MaybeUninit<u8>; NUM_PREFIX_BYTES],
}

impl fmt::Debug for Header {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prefix = self.read_prefix();
        f.debug_struct("Header")
            .field("prefix_size", &self.prefix_size)
            .field("num_children", &self.num_children)
            .field("node_type", &self.node_type)
            .field("prefix", &prefix)
            .finish()
    }
}

impl Header {
    /// Create a new `Header` for an empty node.
    pub fn empty() -> Self {
        Header {
            prefix_size: 0,
            num_children: 0,
            node_type: NodeType::Node4,
            prefix: MaybeUninit::uninit_array(),
        }
    }

    /// Write a new prefix to this header, updating the existing prefix if
    /// present.
    pub fn write_prefix(&mut self, new_bytes: &[u8]) {
        let prefix_size = usize::try_from(self.prefix_size).unwrap();
        if prefix_size < self.prefix.len() {
            // There is still space left in the actual prefix storage
            let bytes_to_write = cmp::min(new_bytes.len(), self.prefix.len() - prefix_size);

            MaybeUninit::write_slice(
                &mut self.prefix[prefix_size..(prefix_size + bytes_to_write)],
                &new_bytes[..bytes_to_write],
            );
        }
        self.prefix_size += u32::try_from(new_bytes.len()).unwrap();
    }

    /// The number of bytes from the start of the prefix.
    ///
    /// # Panics
    ///
    ///  - Panics if the starting point is greater than or equal to the prefix
    ///    size
    ///  - Panics if the prefix size is greater than NUM_PREFIX_BYTES (there are
    ///    bytes represented by the prefix that are not present in memory).
    pub fn ltrim_prefix(&mut self, num_bytes: usize) {
        assert!(
            num_bytes <= usize::try_from(self.prefix_size).unwrap(),
            "Number of bytes to trim [{}] must be less than or equal to prefix size [{}].",
            num_bytes,
            self.prefix_size
        );
        assert!(
            usize::try_from(self.prefix_size).unwrap() <= NUM_PREFIX_BYTES,
            "Cannot trim prefix when the prefix size [{}] is greater than the number of stored \
             bytes [{}].",
            self.prefix_size,
            NUM_PREFIX_BYTES
        );

        self.prefix_size -= u32::try_from(num_bytes).unwrap();
        unsafe {
            // SAFETY:
            //   - self.prefix is valid for writes and aligned
            ptr::copy(
                self.prefix
                    .as_ptr()
                    .offset(isize::try_from(num_bytes).unwrap()),
                self.prefix.as_mut_ptr(),
                usize::try_from(self.prefix_size).unwrap(),
            )
        }
    }

    /// Read the initialized portion of the prefix present in the header.
    ///
    /// The `prefix_size` can be larger than the `read_prefix().len()` because
    /// only NUM_PREFIX_BYTES are stored.
    pub fn read_prefix(&self) -> &[u8] {
        // SAFETY: The array prefix with length `header.prefix_size` is guaranteed to
        // be initialized up to NUM_PREFIX_BYTES bytes (the max length of the array).
        unsafe {
            MaybeUninit::slice_assume_init_ref(
                &self.prefix
                    [..cmp::min(NUM_PREFIX_BYTES, usize::try_from(self.prefix_size).unwrap())],
            )
        }
    }

    /// Compares the compressed path of a node with the key and returns the
    /// number of equal bytes.
    pub fn match_prefix(&self, possible_key: &[u8]) -> u32 {
        let inited_prefix = self.read_prefix();

        let mut num_matching_bytes: u32 = 0;
        for index in 0..cmp::min(inited_prefix.len(), possible_key.len()) {
            if inited_prefix[index] != possible_key[index] {
                break;
            }

            num_matching_bytes += 1;
        }

        if usize::try_from(num_matching_bytes).unwrap() == NUM_PREFIX_BYTES
            && self.prefix_size > num_matching_bytes
        {
            // If we've matched the max number of bytes, but there are still more in the
            // conceptual prefix we just assume that the prefix matches and continue on.
            // This will be checked in the end by a final comparison vs the key stored
            // alongside the value
            cmp::min(self.prefix_size, u32::try_from(possible_key.len()).unwrap())
        } else {
            num_matching_bytes
        }
    }

    /// Return true if the number of children of this node is greater than or
    /// equal to the maximum capacity of this node type.
    pub fn is_full(&self) -> bool {
        self.num_children() >= self.node_type.upper_capacity()
    }

    /// Return the number of children of this node.
    pub fn num_children(&self) -> usize {
        usize::from(self.num_children)
    }
}

impl Default for Header {
    fn default() -> Self {
        Self::empty()
    }
}

/// An opaque pointer to a Node{4,16,48,256}.
///
/// Could be any one of the NodeTypes, need to perform check on the runtime type
/// and then cast to a `NodeRef`.
#[repr(transparent)]
pub struct OpaqueNodePtr<V>(NonNull<Header>, PhantomData<V>);

impl<V> Copy for OpaqueNodePtr<V> {}

impl<V> Clone for OpaqueNodePtr<V> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), PhantomData)
    }
}

impl<V> fmt::Debug for OpaqueNodePtr<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("OpaqueNodePtr").field(&self.0).finish()
    }
}

impl<V> Eq for OpaqueNodePtr<V> {}

impl<V> PartialEq for OpaqueNodePtr<V> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0 && self.1 == other.1
    }
}

impl<V> OpaqueNodePtr<V> {
    /// Return `true` if this Node_ pointer points to the specified concrete
    /// NodeType.
    pub fn is<N: TaggedNode>(&self) -> bool {
        // SAFETY: inner pointer is guaranteed not to be null by `NonNull` wrapper.
        unsafe { (*self.0.as_ptr()).node_type == N::TYPE }
    }

    /// Create a non-opaque node pointer that will eliminate future type
    /// assertions, if the type of the pointed node matches the given
    /// node type.
    pub fn cast<N: TaggedNode>(self) -> Option<NodePtr<N>> {
        if self.is::<N>() {
            // SAFETY: This cast is safe because all Node_ types have `repr(C)`
            // and have the `Header` field first.
            Some(NodePtr(self.0.cast::<N>()))
        } else {
            None
        }
    }

    /// Cast this opaque pointer type an enum that contains a pointer to the
    /// concrete node type.
    pub fn to_node_ptr(self) -> InnerNodePtr<V> {
        let header = self.read();
        match header.node_type {
            NodeType::Node4 => InnerNodePtr::Node4(self.cast::<InnerNode4<V>>().unwrap()),
            NodeType::Node16 => InnerNodePtr::Node16(self.cast::<InnerNode16<V>>().unwrap()),
            NodeType::Node48 => InnerNodePtr::Node48(self.cast::<InnerNode48<V>>().unwrap()),
            NodeType::Node256 => InnerNodePtr::Node256(self.cast::<InnerNode256<V>>().unwrap()),
            NodeType::Leaf => InnerNodePtr::LeafNode(self.cast::<LeafNode<V>>().unwrap()),
        }
    }

    /// Reads the Node from self without moving it. This leaves the memory in
    /// self unchanged.
    pub fn read(self) -> ManuallyDrop<Header> {
        // SAFETY: The non-null requirements of ptr::read is already
        // guaranteed by the NonNull wrapper. The requirements of proper alignment,
        // initialization, validity for reads are required by the construction
        // of the NodePtr type. An OpaqueNodePtr can only be constructed from an earlier
        // instance of a NodePtr.
        ManuallyDrop::new(unsafe { ptr::read(self.0.as_ptr()) })
    }

    /// Write a new value for the header without moving the old value.
    ///
    /// The old value is overwritten without dropping because the Header struct
    /// is Copy.
    pub fn write(self, updated_value: Header) {
        // SAFETY: The requirements of proper alignment and validity for
        // writes are required by the construction of the NodePtr type. An
        // OpaqueNodePtr can only be constructed from an earlier instance of a
        // NodePtr.
        unsafe { ptr::write(self.0.as_ptr(), updated_value) }
    }

    /// Acquires the underlying *mut pointer.
    pub fn to_ptr(self) -> *mut Header {
        self.0.as_ptr()
    }
}

/// An that encapsulates all the types of Nodes
#[derive(Debug)]
pub enum InnerNodePtr<V> {
    /// Node that references between 2 and 4 children
    Node4(NodePtr<InnerNode4<V>>),
    /// Node that references between 5 and 16 children
    Node16(NodePtr<InnerNode16<V>>),
    /// Node that references between 17 and 49 children
    Node48(NodePtr<InnerNode48<V>>),
    /// Node that references between 49 and 256 children
    Node256(NodePtr<InnerNode256<V>>),
    /// Node that contains a single value
    LeafNode(NodePtr<LeafNode<V>>),
}

/// A pointer to a Node{4,16,48,256}.
#[derive(Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct NodePtr<N: TaggedNode>(NonNull<N>);

impl<N: TaggedNode> NodePtr<N> {
    /// Create a safe pointer to a Node_.
    ///
    /// # Safety
    ///
    /// Given pointer must be non-null, aligned, and valid for reads or writes
    /// of a value of N type.
    pub unsafe fn new(ptr: *mut N) -> Self {
        // SAFETY: The safety requirements of this function match the
        // requirements of `NonNull::new_unchecked`.
        unsafe { NodePtr(NonNull::new_unchecked(ptr)) }
    }

    /// Cast node pointer back to an opaque version, losing type information
    pub fn to_opaque(self) -> OpaqueNodePtr<N::Value> {
        // SAFETY: This cast is safe because all Node_ types have `repr(C)`
        // and have the `Header` field first.
        OpaqueNodePtr(self.0.cast::<Header>(), PhantomData)
    }

    /// Reads the Node from self without moving it. This leaves the memory in
    /// self unchanged.
    pub fn read(self) -> ManuallyDrop<N> {
        // SAFETY: The non-null requirements of ptr::read is already
        // guaranteed by the NonNull wrapper. The requirements of proper alignment,
        // initialization, validity for reads are required by the construction
        // of the NodePtr type.
        ManuallyDrop::new(unsafe { ptr::read(self.0.as_ptr()) })
    }

    /// Read the current value value of this node, update it, then write it back
    /// to memory.
    ///
    /// If the given closure panics, then the value will not be updated.
    pub fn update(self, f: impl FnOnce(ManuallyDrop<N>) -> ManuallyDrop<N>) {
        let inner_value = self.read();

        let updated_value = (f)(inner_value);

        // SAFETY: The requirements of proper alignment and validity for
        // writes are required by the construction of the NodePtr type. An
        // OpaqueNodePtr can only be constructed from an earlier instance of a
        // NodePtr.
        unsafe { ptr::write(self.0.as_ptr(), ManuallyDrop::into_inner(updated_value)) }
    }

    /// Acquires the underlying *mut pointer.
    pub fn to_ptr(self) -> *mut N {
        self.0.as_ptr()
    }
}

impl<N: TaggedNode> Clone for NodePtr<N> {
    fn clone(&self) -> Self {
        Self(self.0)
    }
}
impl<N: TaggedNode> Copy for NodePtr<N> {}

impl<N: TaggedNode> From<&mut N> for NodePtr<N> {
    fn from(node_ref: &mut N) -> Self {
        // SAFETY: Pointer is non-null, aligned, and pointing to a valid instance of N
        // because it was constructed from a mutable reference.
        unsafe { NodePtr::new(node_ref as *mut _) }
    }
}

/// All nodes which contain a runtime tag that validates their type.
pub trait TaggedNode {
    /// The runtime type of the node.
    const TYPE: NodeType;

    /// The value type carried by the leaf nodes
    type Value;
}

/// Node that references between 2 and 4 children
#[repr(C)]
pub struct InnerNode4<V> {
    /// The common node fields.
    pub header: Header,
    /// An array that contains single key bytes in the same index as the
    /// `child_pointers` array contains the matching child tree.
    ///
    /// This array will only be initialized for the first `header.num_children`
    /// values.
    pub keys: [MaybeUninit<u8>; 4],
    /// An array that contains the child data.
    ///
    /// This array will only be initialized for the first`header.num_children`
    /// values.
    pub child_pointers: [MaybeUninit<OpaqueNodePtr<V>>; 4],
}

impl<V> Copy for InnerNode4<V> {}

impl<V> Clone for InnerNode4<V> {
    fn clone(&self) -> Self {
        Self {
            header: self.header.clone(),
            keys: self.keys.clone(),
            child_pointers: self.child_pointers.clone(),
        }
    }
}

impl<V> fmt::Debug for InnerNode4<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (keys, child_pointers) = self.initialized_portion();
        f.debug_struct("InnerNode4")
            .field("header", &self.header)
            .field("keys", &keys)
            .field("child_pointers", &child_pointers)
            .finish()
    }
}

impl<V> InnerNode4<V> {
    /// Create an empty `Node4`.
    pub fn empty() -> Self {
        InnerNode4 {
            header: Header::empty(),
            child_pointers: MaybeUninit::uninit_array(),
            keys: MaybeUninit::uninit_array(),
        }
    }

    /// Search through this node for a child node that corresponds to the given
    /// key fragment.
    pub fn lookup_child(&self, key_fragment: u8) -> Option<OpaqueNodePtr<V>> {
        let (keys, child_pointers) = self.initialized_portion();

        for (child_index, key) in keys.iter().enumerate() {
            if *key == key_fragment {
                return Some(child_pointers[child_index]);
            }
        }

        None
    }

    /// Write a new child pointer with key fragment to this inner node.
    ///
    /// # Panics
    ///
    /// Panics when the node is full.
    pub fn write_child(&mut self, key_fragment: u8, child_pointer: OpaqueNodePtr<V>) {
        let child_index = self.header.num_children();
        self.keys[child_index].write(key_fragment);
        self.child_pointers[child_index].write(child_pointer);
        self.header.num_children += 1;
    }

    /// Return an iterator over all the children of this node with their
    /// associated key fragment
    pub fn iter(&self) -> impl Iterator<Item = (u8, OpaqueNodePtr<V>)> + '_ {
        let (keys, child_pointers) = self.initialized_portion();
        keys.iter().copied().zip(child_pointers.iter().copied())
    }

    /// Return the initialized portions of the keys and child pointer arrays.
    pub fn initialized_portion(&self) -> (&[u8], &[OpaqueNodePtr<V>]) {
        // SAFETY: The array prefix with length `header.num_children` is guaranteed to
        // be initialized
        unsafe {
            (
                MaybeUninit::slice_assume_init_ref(&self.keys[0..self.header.num_children()]),
                MaybeUninit::slice_assume_init_ref(
                    &self.child_pointers[0..self.header.num_children()],
                ),
            )
        }
    }

    /// Grow this node into the next larger class, copying over children and
    /// prefix information.
    pub fn grow(self) -> InnerNode16<V> {
        let mut header = self.header;
        header.node_type = InnerNode16::<V>::TYPE;
        let mut keys = MaybeUninit::<u8>::uninit_array::<16>();
        let mut child_pointers = MaybeUninit::<OpaqueNodePtr<V>>::uninit_array::<16>();

        keys[..header.num_children()].copy_from_slice(&self.keys[..header.num_children()]);
        child_pointers[..header.num_children()]
            .copy_from_slice(&self.child_pointers[..header.num_children()]);

        InnerNode16 {
            header,
            keys,
            child_pointers,
        }
    }
}

impl<V> TaggedNode for InnerNode4<V> {
    type Value = V;

    const TYPE: NodeType = NodeType::Node4;
}

/// Node that references between 5 and 16 children
#[repr(C)]
pub struct InnerNode16<V> {
    /// The common node fields.
    pub header: Header,
    /// An array that contains single key bytes in the same index as the
    /// `child_pointers` array contains the matching child tree.
    ///
    /// This array will only be initialized for `header.num_children` values,
    /// starting from the 0 index.
    pub keys: [MaybeUninit<u8>; 16],
    /// An array that contains the child data.
    ///
    /// This array will only be initialized for `header.num_children` values,
    /// starting from the 0 index.
    pub child_pointers: [MaybeUninit<OpaqueNodePtr<V>>; 16],
}

impl<V> Copy for InnerNode16<V> {}

impl<V> Clone for InnerNode16<V> {
    fn clone(&self) -> Self {
        Self {
            header: self.header.clone(),
            keys: self.keys.clone(),
            child_pointers: self.child_pointers.clone(),
        }
    }
}

impl<V> fmt::Debug for InnerNode16<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (keys, child_pointers) = self.initialized_portion();
        f.debug_struct("InnerNode16")
            .field("header", &self.header)
            .field("keys", &keys)
            .field("child_pointers", &child_pointers)
            .finish()
    }
}

impl<V> InnerNode16<V> {
    /// Create an empty `Node16`.
    pub fn empty() -> Self {
        InnerNode16 {
            header: Header {
                node_type: NodeType::Node16,
                ..Header::default()
            },
            child_pointers: MaybeUninit::uninit_array(),
            keys: MaybeUninit::uninit_array(),
        }
    }

    /// Search through this node for a child node that corresponds to the given
    /// key fragment.
    pub fn lookup_child(&self, key_fragment: u8) -> Option<OpaqueNodePtr<V>> {
        let (keys, child_pointers) = self.initialized_portion();

        for (child_index, key) in keys.iter().enumerate() {
            if *key == key_fragment {
                return Some(child_pointers[child_index]);
            }
        }

        None
    }

    /// Write a new child pointer with key fragment to this inner node.
    ///
    /// # Panics
    ///
    /// Panics when the node is full.
    pub fn write_child(&mut self, key_fragment: u8, child_pointer: OpaqueNodePtr<V>) {
        let child_index = self.header.num_children();
        self.keys[child_index].write(key_fragment);
        self.child_pointers[child_index].write(child_pointer);
        self.header.num_children += 1;
    }

    /// Return an iterator over all the children of this node with their
    /// associated key fragment
    pub fn iter(&self) -> impl Iterator<Item = (u8, OpaqueNodePtr<V>)> + '_ {
        let (keys, child_pointers) = self.initialized_portion();

        keys.iter().copied().zip(child_pointers.iter().copied())
    }

    /// Return the initialized portions of the keys and child pointer arrays.
    pub fn initialized_portion(&self) -> (&[u8], &[OpaqueNodePtr<V>]) {
        // SAFETY: The array prefix with length `header.num_children` is guaranteed to
        // be initialized
        unsafe {
            (
                MaybeUninit::slice_assume_init_ref(&self.keys[0..self.header.num_children()]),
                MaybeUninit::slice_assume_init_ref(
                    &self.child_pointers[0..self.header.num_children()],
                ),
            )
        }
    }

    /// Grow this node into the next larger class, copying over children and
    /// prefix information.
    pub fn grow(self) -> InnerNode48<V> {
        let mut header = self.header;
        header.node_type = InnerNode48::<V>::TYPE;
        let mut child_indices = [RestrictedNodeIndex::<48>::EMPTY; 256];
        let mut child_pointers = MaybeUninit::<OpaqueNodePtr<V>>::uninit_array::<48>();

        let (n16_keys, _) = self.initialized_portion();

        for (index, key) in n16_keys.iter().copied().enumerate() {
            // PANIC SAFETY: This `try_from` will not panic because index is guaranteed to
            // be 15 or less because of the length of the `InnerNode16.keys` array.
            child_indices[usize::from(key)] = RestrictedNodeIndex::<48>::try_from(index).unwrap();
        }

        child_pointers[..header.num_children()]
            .copy_from_slice(&self.child_pointers[..header.num_children()]);

        InnerNode48 {
            header,
            child_indices,
            child_pointers,
        }
    }
}

impl<V> TaggedNode for InnerNode16<V> {
    type Value = V;

    const TYPE: NodeType = NodeType::Node16;
}

/// A restricted index only valid from 0 to LIMIT - 1.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(transparent)]
pub struct RestrictedNodeIndex<const LIMIT: u8>(u8);

impl<const LIMIT: u8> RestrictedNodeIndex<LIMIT> {
    /// A placeholder index value that indicates that the index is not occupied
    pub const EMPTY: Self = RestrictedNodeIndex(LIMIT);
}

impl<const LIMIT: u8> From<RestrictedNodeIndex<LIMIT>> for u8 {
    fn from(src: RestrictedNodeIndex<LIMIT>) -> Self {
        src.0
    }
}

impl<const LIMIT: u8> TryFrom<usize> for RestrictedNodeIndex<LIMIT> {
    type Error = TryFromByteError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        if value < usize::from(LIMIT) {
            Ok(RestrictedNodeIndex(value as u8))
        } else {
            Err(TryFromByteError(LIMIT, value))
        }
    }
}

/// The error type returned when attempting to construct an index outside the
/// accepted range of a PartialNodeIndex.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TryFromByteError(u8, usize);

impl fmt::Display for TryFromByteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Input value [{}] is greater than the allowed maximum [{}] for PartialNodeIndex.",
            self.1, self.0
        )
    }
}

impl Error for TryFromByteError {}

/// Node that references between 17 and 49 children
#[derive(Debug)]
#[repr(C)]
pub struct InnerNode48<V> {
    /// The common node fields.
    pub header: Header,
    /// An array that maps key bytes (as the index) to the index value in the
    /// `child_pointers` array.
    ///
    /// All the `child_indices` values are guaranteed to be
    /// `PartialNodeIndex<48>::EMPTY` when the node is constructed.
    pub child_indices: [RestrictedNodeIndex<48>; 256],
    /// For each element in this array, it is assumed to be initialized if there
    /// is a index in the `child_indices` array that points to it
    pub child_pointers: [MaybeUninit<OpaqueNodePtr<V>>; 48],
}

impl<V> Copy for InnerNode48<V> {}

impl<V> Clone for InnerNode48<V> {
    fn clone(&self) -> Self {
        Self {
            header: self.header.clone(),
            child_indices: self.child_indices.clone(),
            child_pointers: self.child_pointers.clone(),
        }
    }
}

impl<V> InnerNode48<V> {
    /// Create an empty `Node48`.
    pub fn empty() -> Self {
        InnerNode48 {
            header: Header {
                node_type: NodeType::Node48,
                ..Header::default()
            },
            child_indices: [RestrictedNodeIndex::<48>::EMPTY; 256],
            child_pointers: MaybeUninit::uninit_array(),
        }
    }

    /// Search through this node for a child node that corresponds to the given
    /// key fragment.
    pub fn lookup_child(&self, key_fragment: u8) -> Option<OpaqueNodePtr<V>> {
        let index = &self.child_indices[key_fragment as usize];
        let child_pointers = self.initialized_child_pointers();
        if *index != RestrictedNodeIndex::<48>::EMPTY {
            Some(child_pointers[usize::from(u8::from(*index))])
        } else {
            None
        }
    }

    /// Write a new child pointer with key fragment to this inner node.
    ///
    /// # Panics
    ///
    /// Panics when the node is full.
    pub fn write_child(&mut self, key_fragment: u8, child_pointer: OpaqueNodePtr<V>) {
        let child_index = self.header.num_children();
        self.child_indices[usize::from(key_fragment)] =
            RestrictedNodeIndex::<48>::try_from(child_index).unwrap();
        self.child_pointers[child_index].write(child_pointer);
        self.header.num_children += 1;
    }

    /// Return an iterator over all the children of this node with their
    /// associated key fragment
    pub fn iter(&self) -> impl Iterator<Item = (u8, OpaqueNodePtr<V>)> + '_ {
        let child_pointers = self.initialized_child_pointers();

        self.child_indices
            .iter()
            .enumerate()
            .filter(|(_, index)| **index != RestrictedNodeIndex::<48>::EMPTY)
            .map(|(key_fragment, index)| {
                (
                    // PANIC SAFETY: The length of the `self.child_indices` array is 256, so the
                    // `key_fragment` derived from `enumerate()` will never exceed the bounds of a
                    // u8
                    u8::try_from(key_fragment).unwrap(),
                    child_pointers[usize::from(u8::from(*index))],
                )
            })
    }

    /// Return the initialized portions of the child pointer array.
    pub fn initialized_child_pointers(&self) -> &[OpaqueNodePtr<V>] {
        // SAFETY: The array prefix with length `header.num_children` is guaranteed to
        // be initialized
        unsafe {
            MaybeUninit::slice_assume_init_ref(&self.child_pointers[0..self.header.num_children()])
        }
    }

    /// Grow this node into the next larger class, copying over children and
    /// prefix information.
    pub fn grow(self) -> InnerNode256<V> {
        let mut header = self.header;
        header.node_type = InnerNode256::<V>::TYPE;
        let mut child_pointers = [None; 256];

        for (key_fragment, child_pointer) in self.iter() {
            child_pointers[usize::from(key_fragment)] = Some(child_pointer);
        }

        InnerNode256 {
            header,
            child_pointers,
        }
    }
}

impl<V> TaggedNode for InnerNode48<V> {
    type Value = V;

    const TYPE: NodeType = NodeType::Node48;
}

/// Node that references between 49 and 256 children
#[derive(Debug)]
#[repr(C)]
pub struct InnerNode256<V> {
    /// The common node fields.
    pub header: Header,
    /// An array that directly maps a key byte (as index) to a child node.
    pub child_pointers: [Option<OpaqueNodePtr<V>>; 256],
}

impl<V> Copy for InnerNode256<V> {}

impl<V> Clone for InnerNode256<V> {
    fn clone(&self) -> Self {
        Self {
            header: self.header.clone(),
            child_pointers: self.child_pointers.clone(),
        }
    }
}

impl<V> InnerNode256<V> {
    /// Create an empty `Node48`.
    pub fn empty() -> Self {
        InnerNode256 {
            header: Header {
                node_type: NodeType::Node256,
                ..Header::default()
            },
            child_pointers: [None; 256],
        }
    }

    /// Search through this node for a child node that corresponds to the given
    /// key fragment.
    pub fn lookup_child(&self, key_fragment: u8) -> Option<OpaqueNodePtr<V>> {
        self.child_pointers[key_fragment as usize]
    }

    /// Write a new child pointer with key fragment to this inner node.
    pub fn write_child(&mut self, key_fragment: u8, child_pointer: OpaqueNodePtr<V>) {
        self.child_pointers[usize::from(key_fragment)] = Some(child_pointer);
        self.header.num_children += 1;
    }

    /// Return an iterator over all the children of this node with their
    /// associated key fragment
    pub fn iter(&self) -> impl Iterator<Item = (u8, OpaqueNodePtr<V>)> + '_ {
        self.child_pointers
            .iter()
            .enumerate()
            .filter_map(|(key_fragment, child_pointer)| {
                child_pointer.map(|child_pointer| {
                    (
                        // PANIC SAFETY: The length of the `self.child_pointers` array is 256, so
                        // the `key_fragment` derived from `enumerate()`
                        // will never exceed the bounds of a u8
                        u8::try_from(key_fragment).unwrap(),
                        child_pointer,
                    )
                })
            })
    }
}

impl<V> TaggedNode for InnerNode256<V> {
    type Value = V;

    const TYPE: NodeType = NodeType::Node256;
}

/// Node that contains a single leaf value.
#[derive(Debug)]
#[repr(C)]
pub struct LeafNode<V> {
    /// The common node fields.
    pub header: Header,
    /// The leaf value.
    pub value: V,
    /// The full key that the `value` was stored with.
    pub key: Box<[u8]>,
}

impl<V> LeafNode<V> {
    /// Create a new leaf node with the given value.
    pub fn new(key: Box<[u8]>, value: V) -> Self {
        LeafNode {
            value,
            key,
            header: Header {
                node_type: NodeType::Leaf,
                ..Header::default()
            },
        }
    }

    /// Check that the provided key is the same one as the stored key.
    pub fn matches_key(&self, possible_key: &[u8]) -> bool {
        self.key.as_ref().eq(possible_key)
    }
}

impl<V> TaggedNode for LeafNode<V> {
    type Value = V;

    const TYPE: NodeType = NodeType::Leaf;
}

#[cfg(test)]
mod tests {
    use std::mem;

    use super::*;

    #[test]
    fn opaque_node_ptr_is_correct() {
        let mut n4 = InnerNode4::<usize>::empty();
        let mut n16 = InnerNode16::<usize>::empty();
        let mut n48 = InnerNode48::<usize>::empty();
        let mut n256 = InnerNode256::<usize>::empty();

        let n4_ptr = NodePtr::from(&mut n4);
        let n16_ptr = NodePtr::from(&mut n16);
        let n48_ptr = NodePtr::from(&mut n48);
        let n256_ptr = NodePtr::from(&mut n256);

        let opaque_n4_ptr = n4_ptr.to_opaque();
        let opaque_n16_ptr = n16_ptr.to_opaque();
        let opaque_n48_ptr = n48_ptr.to_opaque();
        let opaque_n256_ptr = n256_ptr.to_opaque();

        assert!(opaque_n4_ptr.is::<InnerNode4<usize>>());
        assert!(opaque_n16_ptr.is::<InnerNode16<usize>>());
        assert!(opaque_n48_ptr.is::<InnerNode48<usize>>());
        assert!(opaque_n256_ptr.is::<InnerNode256<usize>>());
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn node_sizes() {
        let header_size = mem::size_of::<Header>();
        assert_eq!(header_size, 16);
        // key map: 4 * (1 byte) = 4 bytes
        // child map: 4 * (8 bytes (on 64-bit platform)) = 32
        //
        // 4 bytes of padding are inserted after the `keys` field to align the field to
        // an 8 byte boundary.
        assert_eq!(mem::size_of::<InnerNode4<usize>>() - header_size, 40);
        // key map: 16 * (1 byte) = 16 bytes
        // child map: 16 * (8 bytes (on 64-bit platform)) = 128
        assert_eq!(mem::size_of::<InnerNode16<usize>>() - header_size, 144);
        // key map: 256 * (1 byte) = 256 bytes
        // child map: 48 * (8 bytes (on 64-bit platform)) = 384
        assert_eq!(mem::size_of::<InnerNode48<usize>>() - header_size, 640);
        // child & key map: 256 * (8 bytes (on 64-bit platform)) = 2048
        assert_eq!(mem::size_of::<InnerNode256<usize>>() - header_size, 2048);
    }

    #[test]
    fn node4_lookup() {
        let mut n = InnerNode4::empty();
        let mut l1 = LeafNode::new(vec![].into(), ());
        let mut l2 = LeafNode::new(vec![].into(), ());
        let mut l3 = LeafNode::new(vec![].into(), ());
        let l1_ptr = NodePtr::from(&mut l1).to_opaque();
        let l2_ptr = NodePtr::from(&mut l2).to_opaque();
        let l3_ptr = NodePtr::from(&mut l3).to_opaque();

        assert!(n.lookup_child(123).is_none());

        n.header.num_children = 3;
        n.keys[0].write(3);
        n.keys[1].write(123);
        n.keys[2].write(1);

        n.child_pointers[0].write(l1_ptr);
        n.child_pointers[1].write(l2_ptr);
        n.child_pointers[2].write(l3_ptr);

        assert_eq!(n.lookup_child(123), Some(l2_ptr));
    }

    #[test]
    fn node4_write_child() {
        let mut n = InnerNode4::empty();
        let mut l1 = LeafNode::new(vec![].into(), ());
        let mut l2 = LeafNode::new(vec![].into(), ());
        let mut l3 = LeafNode::new(vec![].into(), ());
        let l1_ptr = NodePtr::from(&mut l1).to_opaque();
        let l2_ptr = NodePtr::from(&mut l2).to_opaque();
        let l3_ptr = NodePtr::from(&mut l3).to_opaque();

        n.write_child(3, l1_ptr);
        n.write_child(123, l2_ptr);
        n.write_child(1, l3_ptr);

        assert_eq!(n.lookup_child(3), Some(l1_ptr));
        assert_eq!(n.lookup_child(123), Some(l2_ptr));
        assert_eq!(n.lookup_child(1), Some(l3_ptr));
    }

    #[test]
    fn node4_grow() {
        let mut n4 = InnerNode4::empty();
        let mut l1 = LeafNode::new(vec![].into(), ());
        let mut l2 = LeafNode::new(vec![].into(), ());
        let mut l3 = LeafNode::new(vec![].into(), ());
        let l1_ptr = NodePtr::from(&mut l1).to_opaque();
        let l2_ptr = NodePtr::from(&mut l2).to_opaque();
        let l3_ptr = NodePtr::from(&mut l3).to_opaque();

        n4.write_child(3, l1_ptr);
        n4.write_child(123, l2_ptr);
        n4.write_child(1, l3_ptr);

        let n16 = n4.grow();

        assert_eq!(n16.lookup_child(3), Some(l1_ptr));
        assert_eq!(n16.lookup_child(123), Some(l2_ptr));
        assert_eq!(n16.lookup_child(1), Some(l3_ptr));
        assert_eq!(n16.lookup_child(4), None);
    }

    #[test]
    fn node4_iterate() {
        let mut n4 = InnerNode4::empty();
        let mut l1 = LeafNode::new(vec![].into(), ());
        let mut l2 = LeafNode::new(vec![].into(), ());
        let mut l3 = LeafNode::new(vec![].into(), ());
        let l1_ptr = NodePtr::from(&mut l1).to_opaque();
        let l2_ptr = NodePtr::from(&mut l2).to_opaque();
        let l3_ptr = NodePtr::from(&mut l3).to_opaque();

        n4.write_child(3, l1_ptr);
        n4.write_child(123, l2_ptr);
        n4.write_child(1, l3_ptr);

        let pairs = n4.iter().collect::<Vec<_>>();
        assert!(pairs
            .iter()
            .find(|(key_fragment, ptr)| *key_fragment == 3 && *ptr == l1_ptr)
            .is_some());
        assert!(pairs
            .iter()
            .find(|(key_fragment, ptr)| *key_fragment == 123 && *ptr == l2_ptr)
            .is_some());
        assert!(pairs
            .iter()
            .find(|(key_fragment, ptr)| *key_fragment == 1 && *ptr == l3_ptr)
            .is_some());
    }

    #[test]
    fn node16_lookup() {
        let mut n = InnerNode16::empty();
        let mut l1 = LeafNode::new(Box::new([]), ());
        let mut l2 = LeafNode::new(Box::new([]), ());
        let mut l3 = LeafNode::new(Box::new([]), ());
        let l1_ptr = NodePtr::from(&mut l1).to_opaque();
        let l2_ptr = NodePtr::from(&mut l2).to_opaque();
        let l3_ptr = NodePtr::from(&mut l3).to_opaque();

        assert!(n.lookup_child(123).is_none());

        n.header.num_children = 3;
        n.keys[0].write(3);
        n.keys[1].write(123);
        n.keys[2].write(1);

        n.child_pointers[0].write(l1_ptr);
        n.child_pointers[1].write(l2_ptr);
        n.child_pointers[2].write(l3_ptr);

        assert_eq!(n.lookup_child(123), Some(l2_ptr));
    }

    #[test]
    fn node16_write_child() {
        let mut n = InnerNode16::empty();
        let mut l1 = LeafNode::new(vec![].into(), ());
        let mut l2 = LeafNode::new(vec![].into(), ());
        let mut l3 = LeafNode::new(vec![].into(), ());
        let l1_ptr = NodePtr::from(&mut l1).to_opaque();
        let l2_ptr = NodePtr::from(&mut l2).to_opaque();
        let l3_ptr = NodePtr::from(&mut l3).to_opaque();

        n.write_child(3, l1_ptr);
        n.write_child(123, l2_ptr);
        n.write_child(1, l3_ptr);

        assert_eq!(n.lookup_child(3), Some(l1_ptr));
        assert_eq!(n.lookup_child(123), Some(l2_ptr));
        assert_eq!(n.lookup_child(1), Some(l3_ptr));
    }

    #[test]
    fn node16_grow() {
        let mut n16 = InnerNode16::empty();
        let mut l1 = LeafNode::new(vec![].into(), ());
        let mut l2 = LeafNode::new(vec![].into(), ());
        let mut l3 = LeafNode::new(vec![].into(), ());
        let l1_ptr = NodePtr::from(&mut l1).to_opaque();
        let l2_ptr = NodePtr::from(&mut l2).to_opaque();
        let l3_ptr = NodePtr::from(&mut l3).to_opaque();

        n16.write_child(3, l1_ptr);
        n16.write_child(123, l2_ptr);
        n16.write_child(1, l3_ptr);

        let n48 = n16.grow();

        assert_eq!(n48.lookup_child(3), Some(l1_ptr));
        assert_eq!(n48.lookup_child(123), Some(l2_ptr));
        assert_eq!(n48.lookup_child(1), Some(l3_ptr));
        assert_eq!(n48.lookup_child(4), None);
    }

    #[test]
    fn node16_iterate() {
        let mut n = InnerNode16::empty();
        let mut l1 = LeafNode::new(vec![].into(), ());
        let mut l2 = LeafNode::new(vec![].into(), ());
        let mut l3 = LeafNode::new(vec![].into(), ());
        let l1_ptr = NodePtr::from(&mut l1).to_opaque();
        let l2_ptr = NodePtr::from(&mut l2).to_opaque();
        let l3_ptr = NodePtr::from(&mut l3).to_opaque();

        n.write_child(3, l1_ptr);
        n.write_child(123, l2_ptr);
        n.write_child(1, l3_ptr);

        let pairs = n.iter().collect::<Vec<_>>();
        assert!(pairs
            .iter()
            .find(|(key_fragment, ptr)| *key_fragment == 3 && *ptr == l1_ptr)
            .is_some());
        assert!(pairs
            .iter()
            .find(|(key_fragment, ptr)| *key_fragment == 123 && *ptr == l2_ptr)
            .is_some());
        assert!(pairs
            .iter()
            .find(|(key_fragment, ptr)| *key_fragment == 1 && *ptr == l3_ptr)
            .is_some());
    }

    #[test]
    fn node48_lookup() {
        let mut n = InnerNode48::empty();
        let mut l1 = LeafNode::new(Box::new([]), ());
        let mut l2 = LeafNode::new(Box::new([]), ());
        let mut l3 = LeafNode::new(Box::new([]), ());
        let l1_ptr = NodePtr::from(&mut l1).to_opaque();
        let l2_ptr = NodePtr::from(&mut l2).to_opaque();
        let l3_ptr = NodePtr::from(&mut l3).to_opaque();

        assert!(n.lookup_child(123).is_none());

        n.header.num_children = 3;

        n.child_indices[1] = 2.try_into().unwrap();
        n.child_indices[3] = 0.try_into().unwrap();
        n.child_indices[123] = 1.try_into().unwrap();

        n.child_pointers[0].write(l1_ptr);
        n.child_pointers[1].write(l2_ptr);
        n.child_pointers[2].write(l3_ptr);

        assert_eq!(n.lookup_child(123), Some(l2_ptr));
    }

    #[test]
    fn node48_write_child() {
        let mut n = InnerNode48::empty();
        let mut l1 = LeafNode::new(vec![].into(), ());
        let mut l2 = LeafNode::new(vec![].into(), ());
        let mut l3 = LeafNode::new(vec![].into(), ());
        let l1_ptr = NodePtr::from(&mut l1).to_opaque();
        let l2_ptr = NodePtr::from(&mut l2).to_opaque();
        let l3_ptr = NodePtr::from(&mut l3).to_opaque();

        n.write_child(3, l1_ptr);
        n.write_child(123, l2_ptr);
        n.write_child(1, l3_ptr);

        assert_eq!(n.lookup_child(3), Some(l1_ptr));
        assert_eq!(n.lookup_child(123), Some(l2_ptr));
        assert_eq!(n.lookup_child(1), Some(l3_ptr));
    }

    #[test]
    fn node48_grow() {
        let mut n48 = InnerNode48::empty();
        let mut l1 = LeafNode::new(vec![].into(), ());
        let mut l2 = LeafNode::new(vec![].into(), ());
        let mut l3 = LeafNode::new(vec![].into(), ());
        let l1_ptr = NodePtr::from(&mut l1).to_opaque();
        let l2_ptr = NodePtr::from(&mut l2).to_opaque();
        let l3_ptr = NodePtr::from(&mut l3).to_opaque();

        n48.write_child(3, l1_ptr);
        n48.write_child(123, l2_ptr);
        n48.write_child(1, l3_ptr);

        let n256 = n48.grow();

        assert_eq!(n256.lookup_child(3), Some(l1_ptr));
        assert_eq!(n256.lookup_child(123), Some(l2_ptr));
        assert_eq!(n256.lookup_child(1), Some(l3_ptr));
        assert_eq!(n256.lookup_child(4), None);
    }

    #[test]
    fn node48_iterate() {
        let mut n = InnerNode48::empty();
        let mut l1 = LeafNode::new(vec![].into(), ());
        let mut l2 = LeafNode::new(vec![].into(), ());
        let mut l3 = LeafNode::new(vec![].into(), ());
        let l1_ptr = NodePtr::from(&mut l1).to_opaque();
        let l2_ptr = NodePtr::from(&mut l2).to_opaque();
        let l3_ptr = NodePtr::from(&mut l3).to_opaque();

        n.write_child(3, l1_ptr);
        n.write_child(123, l2_ptr);
        n.write_child(1, l3_ptr);

        let pairs = n.iter().collect::<Vec<_>>();
        assert!(pairs
            .iter()
            .find(|(key_fragment, ptr)| *key_fragment == 3 && *ptr == l1_ptr)
            .is_some());
        assert!(pairs
            .iter()
            .find(|(key_fragment, ptr)| *key_fragment == 123 && *ptr == l2_ptr)
            .is_some());
        assert!(pairs
            .iter()
            .find(|(key_fragment, ptr)| *key_fragment == 1 && *ptr == l3_ptr)
            .is_some());
    }

    #[test]
    fn node256_lookup() {
        let mut n = InnerNode256::empty();
        let mut l1 = LeafNode::new(Box::new([]), ());
        let mut l2 = LeafNode::new(Box::new([]), ());
        let mut l3 = LeafNode::new(Box::new([]), ());
        let l1_ptr = NodePtr::from(&mut l1).to_opaque();
        let l2_ptr = NodePtr::from(&mut l2).to_opaque();
        let l3_ptr = NodePtr::from(&mut l3).to_opaque();

        assert!(n.lookup_child(123).is_none());

        n.header.num_children = 3;

        n.child_pointers[1] = Some(l1_ptr);
        n.child_pointers[123] = Some(l2_ptr);
        n.child_pointers[3] = Some(l3_ptr);

        assert_eq!(n.lookup_child(123), Some(l2_ptr));
    }

    #[test]
    fn node256_write_child() {
        let mut n = InnerNode256::empty();
        let mut l1 = LeafNode::new(vec![].into(), ());
        let mut l2 = LeafNode::new(vec![].into(), ());
        let mut l3 = LeafNode::new(vec![].into(), ());
        let l1_ptr = NodePtr::from(&mut l1).to_opaque();
        let l2_ptr = NodePtr::from(&mut l2).to_opaque();
        let l3_ptr = NodePtr::from(&mut l3).to_opaque();

        n.write_child(3, l1_ptr);
        n.write_child(123, l2_ptr);
        n.write_child(1, l3_ptr);

        assert_eq!(n.lookup_child(3), Some(l1_ptr));
        assert_eq!(n.lookup_child(123), Some(l2_ptr));
        assert_eq!(n.lookup_child(1), Some(l3_ptr));
    }

    #[test]
    fn node256_iterate() {
        let mut n = InnerNode256::empty();
        let mut l1 = LeafNode::new(vec![].into(), ());
        let mut l2 = LeafNode::new(vec![].into(), ());
        let mut l3 = LeafNode::new(vec![].into(), ());
        let l1_ptr = NodePtr::from(&mut l1).to_opaque();
        let l2_ptr = NodePtr::from(&mut l2).to_opaque();
        let l3_ptr = NodePtr::from(&mut l3).to_opaque();

        n.write_child(3, l1_ptr);
        n.write_child(123, l2_ptr);
        n.write_child(1, l3_ptr);

        let pairs = n.iter().collect::<Vec<_>>();
        assert!(pairs
            .iter()
            .find(|(key_fragment, ptr)| *key_fragment == 3 && *ptr == l1_ptr)
            .is_some());
        assert!(pairs
            .iter()
            .find(|(key_fragment, ptr)| *key_fragment == 123 && *ptr == l2_ptr)
            .is_some());
        assert!(pairs
            .iter()
            .find(|(key_fragment, ptr)| *key_fragment == 1 && *ptr == l3_ptr)
            .is_some());
    }

    #[test]
    fn header_read_write_prefix() {
        let mut h = Header::empty();

        assert_eq!(h.prefix_size, 0);
        assert_eq!(h.read_prefix(), &[]);

        h.write_prefix(&[1, 2, 3]);

        assert_eq!(h.prefix_size, 3);
        assert_eq!(h.read_prefix(), &[1, 2, 3]);

        h.write_prefix(&[4, 5, 6]);

        assert_eq!(h.prefix_size, 6);
        assert_eq!(h.read_prefix(), &[1, 2, 3, 4, 5, 6]);

        h.write_prefix(&[7, 8]);

        assert_eq!(h.prefix_size, 8);
        assert_eq!(h.read_prefix(), &[1, 2, 3, 4, 5, 6, 7, 8]);

        h.write_prefix(&[9, 10, 11, 12]);

        assert_eq!(h.prefix_size, 12);
        assert_eq!(h.read_prefix(), &[1, 2, 3, 4, 5, 6, 7, 8]);

        h.write_prefix(&[]);

        assert_eq!(h.prefix_size, 12);
        assert_eq!(h.read_prefix(), &[1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn header_check_prefix() {
        let mut h = Header::empty();

        h.write_prefix(&[1, 2, 3]);

        assert_eq!(h.match_prefix(&[1, 2, 3]), 3);
        assert_eq!(h.match_prefix(&[0, 1, 2]), 0);
        assert_eq!(h.match_prefix(&[1, 2]), 2);
        assert_eq!(h.match_prefix(&[]), 0);

        h.write_prefix(&[4, 5, 6, 7, 8, 9, 10, 11, 12]);

        assert_eq!(h.match_prefix(&[1, 2, 3]), 3);
        assert_eq!(h.match_prefix(&[0, 1, 2]), 0);
        assert_eq!(h.match_prefix(&[1, 2]), 2);
        assert_eq!(h.match_prefix(&[]), 0);

        assert_eq!(h.match_prefix(&[1, 2, 3, 4, 5, 6, 7, 8]), 8);
        assert_eq!(h.match_prefix(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]), 12);
        assert_eq!(
            h.match_prefix(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]),
            12
        );
        assert_eq!(
            h.match_prefix(&[1, 2, 3, 4, 5, 6, 7, 8, 100, 200, 254, 255]),
            12
        );
    }

    #[test]
    fn header_delete_prefix() {
        let mut h = Header::empty();
        h.write_prefix(&[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(h.read_prefix(), &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(h.prefix_size, 8);

        h.ltrim_prefix(0);
        assert_eq!(h.read_prefix(), &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(h.prefix_size, 8);

        h.ltrim_prefix(3);
        assert_eq!(h.read_prefix(), &[4, 5, 6, 7, 8]);
        assert_eq!(h.prefix_size, 5);

        h.ltrim_prefix(1);
        assert_eq!(h.read_prefix(), &[5, 6, 7, 8]);
        assert_eq!(h.prefix_size, 4);

        h.ltrim_prefix(4);
        assert_eq!(h.read_prefix(), &[]);
        assert_eq!(h.prefix_size, 0);
    }

    #[test]
    #[should_panic]
    fn header_ltrim_prefix_too_many_bytes_panic() {
        let mut h = Header::empty();
        h.write_prefix(&[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(h.read_prefix(), &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(h.prefix_size, 8);

        h.ltrim_prefix(10);
    }

    #[test]
    #[should_panic]
    fn header_ltrim_prefix_non_stored_bytes_panic() {
        let mut h = Header::empty();
        h.write_prefix(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]);
        assert_eq!(h.read_prefix(), &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(h.prefix_size, 8);

        h.ltrim_prefix(0);
    }

    #[test]
    fn header_is_full() {
        let mut n = InnerNode4::empty();

        assert!(!n.header.is_full());

        let mut l1 = LeafNode::new(Box::new([]), ());
        let mut l2 = LeafNode::new(Box::new([]), ());
        let mut l3 = LeafNode::new(Box::new([]), ());
        let mut l4 = LeafNode::new(Box::new([]), ());

        {
            let l1_ptr = NodePtr::from(&mut l1).to_opaque();
            let l2_ptr = NodePtr::from(&mut l2).to_opaque();
            let l3_ptr = NodePtr::from(&mut l3).to_opaque();
            let l4_ptr = NodePtr::from(&mut l4).to_opaque();

            n.write_child(0, l1_ptr);
            n.write_child(1, l2_ptr);
            n.write_child(2, l3_ptr);

            assert!(!n.header.is_full());

            n.write_child(3, l4_ptr);

            assert!(n.header.is_full());
        }
    }
}
