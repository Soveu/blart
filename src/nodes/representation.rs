//! Trie node representation

pub use self::iterators::*;
use crate::{tagged_pointer::TaggedPointer, AsBytes, InnerNodeIter};
use std::{
    arch::x86_64::{
        __m128i, _mm_cmpeq_epi8, _mm_cmplt_epi8, _mm_loadu_si128, _mm_movemask_epi8, _mm_set1_epi8,
    },
    borrow::Borrow,
    cmp::Ordering,
    error::Error,
    fmt,
    hash::Hash,
    intrinsics::assume,
    marker::PhantomData,
    mem::{self, ManuallyDrop, MaybeUninit},
    ops::Range,
    ptr::{self, NonNull},
    simd::{u8x16, SimdPartialEq, SimdPartialOrd, Simd},
};
use tinyvec::TinyVec;

mod iterators;

#[cfg(test)]
mod tests;

/// The number of bytes stored for path compression in the node header.
pub const NUM_PREFIX_BYTES: usize = 14;

/// The representation of inner nodes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NodeType {
    /// Node that references between 2 and 4 children
    Node4 = 0b000,
    /// Node that references between 5 and 16 children
    Node16 = 0b001, // 0b001
    /// Node that references between 17 and 49 children
    Node48 = 0b010, // 0b010
    /// Node that references between 49 and 256 children
    Node256 = 0b011, // 0b011
    /// Node that contains a single value
    Leaf = 0b100, // 0b100
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

    /// Attempt to convert a u8 value to a [`NodeType`], returning None if there
    /// is no match.
    pub const fn from_u8(src: u8) -> NodeType {
        unsafe { std::mem::transmute::<u8, NodeType>(src) }
    }

    /// Return true if an [`InnerNode`] with the given [`NodeType`] and
    /// specified number of children should be shrunk.
    ///
    /// # Panics
    ///  - Panics if `node_type` equals [`NodeType::Leaf`]
    pub fn should_shrink_inner_node(self, num_children: usize) -> bool {
        match self {
            NodeType::Node4 => false,
            NodeType::Node16 => num_children <= 4,
            NodeType::Node48 => num_children <= 16,
            NodeType::Node256 => num_children <= 48,
            NodeType::Leaf => panic!("cannot shrink leaf"),
        }
    }

    /// Return the range of number of children that each node type accepts.
    pub const fn capacity_range(self) -> Range<usize> {
        match self {
            NodeType::Node4 => Range { start: 1, end: 5 },
            NodeType::Node16 => Range { start: 5, end: 17 },
            NodeType::Node48 => Range { start: 17, end: 49 },
            NodeType::Node256 => Range {
                start: 49,
                end: 256,
            },
            NodeType::Leaf => Range { start: 0, end: 0 },
        }
    }
}

/// The common header for all inner nodes
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Header {
    /// Number of children of this inner node. This field has no meaning for
    /// a leaf node.
    pub num_children: u16,
    /// The key prefix for this node.
    pub prefix: TinyVec<[u8; NUM_PREFIX_BYTES]>,
}

impl Header {
    /// Create a new `Header` for an empty node.
    pub fn empty() -> Self {
        Header {
            num_children: 0,
            prefix: TinyVec::new(),
        }
    }

    /// Write prefix bytes to this header, appending to existing bytes if
    /// present.
    pub fn extend_prefix(&mut self, new_bytes: &[u8]) {
        self.prefix.extend(new_bytes.iter().copied());
    }

    /// Write bytes to the start of the key prefix.
    pub fn prepend_prefix(&mut self, new_bytes: &[u8]) {
        self.prefix
            .splice(0..0, new_bytes.iter().copied())
            .for_each(|_| ());
    }

    /// Remove the specified number of bytes from the start of the prefix.
    ///
    /// # Panics
    ///
    ///  - Panics if the number of bytes to remove is greater than the prefix
    ///    size.
    pub fn ltrim_prefix(&mut self, num_bytes: usize) {
        // this is an explicit match instead of a direct `self.prefix.drain` because it
        // is more efficient. See `TinyVec::drain` documentation.
        match self.prefix {
            TinyVec::Inline(ref mut vec) => {
                vec.drain(..num_bytes).for_each(|_| ());
            },
            TinyVec::Heap(ref mut vec) => {
                vec.drain(..num_bytes).for_each(|_| ());
            },
        }
    }

    /// Read the initialized portion of the prefix present in the header.
    pub fn read_prefix(&self) -> &[u8] {
        self.prefix.as_ref()
    }

    /// Return the number of bytes in the prefix.
    pub fn prefix_size(&self) -> usize {
        self.prefix.len()
    }

    /// Compares the compressed path of a node with the key and returns the
    /// number of equal bytes.
    pub fn match_prefix(&self, possible_key: &[u8]) -> usize {
        self.read_prefix()
            .iter()
            .zip(possible_key)
            .take_while(|(a, b)| **a == **b)
            .count()
    }

    /// Return the number of children of this node.
    pub fn num_children(&self) -> usize {
        usize::from(self.num_children)
    }
}

/// A placeholder type that has the required amount of alignment.
///
/// An alignment of 8 gives us 3 unused bits in any pointer to this type.
#[derive(Debug)]
#[repr(align(8))]
struct OpaqueValue;

/// An opaque pointer to a [`Node`].
///
/// Could be any one of the NodeTypes, need to perform check on the runtime type
/// and then cast to a [`NodePtr`].
#[repr(transparent)]
pub struct OpaqueNodePtr<K, V>(TaggedPointer<OpaqueValue, 3>, PhantomData<(K, V)>);

impl<K, V> Copy for OpaqueNodePtr<K, V> {}

impl<K, V> Clone for OpaqueNodePtr<K, V> {
    fn clone(&self) -> Self {
        Self(self.0, PhantomData)
    }
}

impl<K, V> fmt::Debug for OpaqueNodePtr<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("OpaqueNodePtr").field(&self.0).finish()
    }
}

impl<K, V> fmt::Pointer for OpaqueNodePtr<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Pointer::fmt(&self.0, f)
    }
}

impl<K, V> Eq for OpaqueNodePtr<K, V> {}

impl<K, V> PartialEq for OpaqueNodePtr<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<K, V> Hash for OpaqueNodePtr<K, V> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl<K, V> OpaqueNodePtr<K, V> {
    /// Construct a new opaque node pointer from an existing non-null node
    /// pointer.
    pub fn new<N>(pointer: NonNull<N>) -> Self
    where
        N: Node<Value = V>,
    {
        let mut tagged_ptr = TaggedPointer::from(pointer).cast::<OpaqueValue>();
        tagged_ptr.set_data(N::TYPE as usize);

        OpaqueNodePtr(tagged_ptr, PhantomData)
    }

    /// Return `true` if this Node_ pointer points to the specified concrete
    /// [`NodeType`].
    pub fn is<N: Node>(&self) -> bool {
        self.0.to_data() == usize::from(N::TYPE as u8)
    }

    /// Create a non-opaque node pointer that will eliminate future type
    /// assertions, if the type of the pointed node matches the given
    /// node type.
    pub fn cast<N: Node>(self) -> Option<NodePtr<N>> {
        if self.is::<N>() {
            Some(NodePtr(self.0.cast::<N>().into()))
        } else {
            None
        }
    }

    /// Cast this opaque pointer type an enum that contains a pointer to the
    /// concrete node type.
    pub fn to_node_ptr(self) -> ConcreteNodePtr<K, V> {
        match self.node_type() {
            NodeType::Node4 => {
                ConcreteNodePtr::Node4(NodePtr(self.0.cast::<InnerNode4<K, V>>().into()))
            },
            NodeType::Node16 => {
                ConcreteNodePtr::Node16(NodePtr(self.0.cast::<InnerNode16<K, V>>().into()))
            },
            NodeType::Node48 => {
                ConcreteNodePtr::Node48(NodePtr(self.0.cast::<InnerNode48<K, V>>().into()))
            },
            NodeType::Node256 => {
                ConcreteNodePtr::Node256(NodePtr(self.0.cast::<InnerNode256<K, V>>().into()))
            },
            NodeType::Leaf => {
                ConcreteNodePtr::LeafNode(NodePtr(self.0.cast::<LeafNode<K, V>>().into()))
            },
        }
    }

    /// Retrieve the runtime node type information.
    pub fn node_type(self) -> NodeType {
        // PANIC SAFETY: We know that we can convert the usize into a `NodeType` because
        // we have only stored `NodeType` values into this pointer
        NodeType::from_u8(self.0.to_data() as u8)
    }

    /// Get a mutable reference to the header if the underlying node has a
    /// header field, otherwise return `None`.
    ///
    /// # Safety
    ///  - You must enforce Rust’s aliasing rules, since the returned lifetime
    ///    'h is arbitrarily chosen and does not necessarily reflect the actual
    ///    lifetime of the data. In particular, for the duration of this
    ///    lifetime, the memory the pointer points to must not get accessed
    ///    (read or written) through any other pointer.
    pub(crate) unsafe fn header_mut<'h>(self) -> Option<&'h mut Header> {
        let header_ptr = match self.node_type() {
            NodeType::Node4 => {
                let node_ptr = self.0.cast::<InnerNode4<K, V>>().to_ptr();

                ptr::addr_of_mut!((*node_ptr).header)
            },
            NodeType::Node16 => {
                let node_ptr = self.0.cast::<InnerNode16<K, V>>().to_ptr();

                ptr::addr_of_mut!((*node_ptr).header)
            },
            NodeType::Node48 => {
                let node_ptr = self.0.cast::<InnerNode48<K, V>>().to_ptr();

                ptr::addr_of_mut!((*node_ptr).header)
            },
            NodeType::Node256 => {
                let node_ptr = self.0.cast::<InnerNode256<K, V>>().to_ptr();

                ptr::addr_of_mut!((*node_ptr).header)
            },
            NodeType::Leaf => {
                return None;
            },
        };

        // SAFETY: The pointer is properly aligned and points to a initialized instance
        // of Header that is dereferenceable. The lifetime safety requirements are
        // passed up to the caller of this function.
        Some(unsafe { &mut *header_ptr })
    }

    pub(crate) unsafe fn header_mut_uncheked<'h>(self) -> &'h mut Header {
        unsafe { &mut *self.0.cast::<Header>().to_ptr() }
    }
}

/// An enum that encapsulates pointers to every type of Node
pub enum ConcreteNodePtr<K, V> {
    /// Node that references between 2 and 4 children
    Node4(NodePtr<InnerNode4<K, V>>),
    /// Node that references between 5 and 16 children
    Node16(NodePtr<InnerNode16<K, V>>),
    /// Node that references between 17 and 49 children
    Node48(NodePtr<InnerNode48<K, V>>),
    /// Node that references between 49 and 256 children
    Node256(NodePtr<InnerNode256<K, V>>),
    /// Node that contains a single value
    LeafNode(NodePtr<LeafNode<K, V>>),
}

impl<K, V> fmt::Debug for ConcreteNodePtr<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Node4(arg0) => f.debug_tuple("Node4").field(arg0).finish(),
            Self::Node16(arg0) => f.debug_tuple("Node16").field(arg0).finish(),
            Self::Node48(arg0) => f.debug_tuple("Node48").field(arg0).finish(),
            Self::Node256(arg0) => f.debug_tuple("Node256").field(arg0).finish(),
            Self::LeafNode(arg0) => f.debug_tuple("LeafNode").field(arg0).finish(),
        }
    }
}

/// A pointer to a [`Node`].
#[repr(transparent)]
pub struct NodePtr<N: Node>(NonNull<N>);

impl<N: Node> NodePtr<N> {
    /// Create a safe pointer to a [`Node`].
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

    /// Allocate the given [`Node`] on the [`std::alloc::Global`] heap and
    /// return a [`NodePtr`] that wrap the raw pointer.
    pub fn allocate_node_ptr(node: N) -> Self {
        // SAFETY: The pointer from [`Box::into_raw`] is non-null, aligned, and valid
        // for reads and writes of the [`Node`] `N`.
        unsafe { NodePtr::new(Box::into_raw(Box::new(node))) }
    }

    /// Deallocate a [`Node`] object created with the
    /// [`NodePtr::allocate_node_ptr`] function.
    ///
    /// # Safety
    ///
    ///  - This function can only be called once for a given node object.
    #[must_use]
    pub unsafe fn deallocate_node_ptr(node: Self) -> N {
        // SAFETY: Covered by safety condition on functiom
        unsafe { *Box::from_raw(node.to_ptr()) }
    }

    /// Moves `new_value` into the referenced `dest`, returning the previous
    /// `dest` value.
    ///
    /// Neither value is dropped.
    ///
    /// # Safety
    ///  - The node the `dest` pointers points to must not get accessed (read or
    ///    written) through any other pointers concurrent to this modification.
    pub unsafe fn replace(dest: Self, new_value: N) -> N {
        // SAFETY: The lifetime of the `dest` reference is restricted to this function,
        // and the referenced node is not accessed by the safety doc on the containing
        // function.
        let dest = unsafe { dest.as_mut() };
        mem::replace(dest, new_value)
    }

    /// Cast node pointer back to an opaque version, losing type information
    pub fn to_opaque(self) -> OpaqueNodePtr<N::Key, N::Value> {
        OpaqueNodePtr::new(self.0)
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

    /// Returns a shared reference to the value.
    ///
    /// # Safety
    ///  - You must enforce Rust’s aliasing rules, since the returned lifetime
    ///    'a is arbitrarily chosen and does not necessarily reflect the actual
    ///    lifetime of the data. In particular, for the duration of this
    ///    lifetime, the memory the pointer points to must not get mutated
    ///    (except inside UnsafeCell).
    pub unsafe fn as_ref<'a>(self) -> &'a N {
        // SAFETY: The pointer is properly aligned and points to a initialized instance
        // of N that is dereferenceable. The lifetime safety requirements are passed up
        // to the invoked of this function.
        unsafe { self.0.as_ref() }
    }

    /// Returns a unique mutable reference to the node.
    ///
    /// # Safety
    ///  - You must enforce Rust’s aliasing rules, since the returned lifetime
    ///    'a is arbitrarily chosen and does not necessarily reflect the actual
    ///    lifetime of the node. In particular, for the duration of this
    ///    lifetime, the node the pointer points to must not get accessed (read
    ///    or written) through any other pointer.
    pub unsafe fn as_mut<'a>(mut self) -> &'a mut N {
        // SAFETY: The pointer is properly aligned and points to a initialized instance
        // of N that is dereferenceable. The lifetime safety requirements are passed up
        // to the invoked of this function.
        unsafe { self.0.as_mut() }
    }

    /// Acquires the underlying *mut pointer.
    pub fn to_ptr(self) -> *mut N {
        self.0.as_ptr()
    }
}

impl<K, V> NodePtr<LeafNode<K, V>> {
    /// Returns a shared reference to the key and value of the pointed to
    /// [`LeafNode`].
    ///
    /// # Safety
    ///  - You must enforce Rust’s aliasing rules, since the returned lifetime
    ///    'a is arbitrarily chosen and does not necessarily reflect the actual
    ///    lifetime of the data. In particular, for the duration of this
    ///    lifetime, the memory the pointer points to must not get mutated
    ///    (except inside UnsafeCell).
    pub unsafe fn as_key_value_ref<'a>(self) -> (&'a K, &'a V) {
        // SAFETY: Safety requirements are covered by the containing function.
        let leaf = unsafe { self.as_ref() };

        (leaf.key_ref(), leaf.value_ref())
    }

    /// Returns a unique mutable reference to the key and value of the pointed
    /// to [`LeafNode`].
    ///
    /// # Safety
    ///  - You must enforce Rust’s aliasing rules, since the returned lifetime
    ///    'a is arbitrarily chosen and does not necessarily reflect the actual
    ///    lifetime of the node. In particular, for the duration of this
    ///    lifetime, the node the pointer points to must not get accessed (read
    ///    or written) through any other pointer.
    pub unsafe fn as_key_ref_value_mut<'a>(self) -> (&'a K, &'a mut V) {
        // SAFETY: Safety requirements are covered by the containing function.
        let leaf = unsafe { self.as_mut() };

        let (key, value) = leaf.entry_mut();
        (key, value)
    }

    /// Returns a unique mutable reference to the key and value of the pointed
    /// to [`LeafNode`].
    ///
    /// # Safety
    ///  - You must enforce Rust’s aliasing rules, since the returned lifetime
    ///    'a is arbitrarily chosen and does not necessarily reflect the actual
    ///    lifetime of the data. In particular, for the duration of this
    ///    lifetime, the memory the pointer points to must not get mutated
    ///    (except inside UnsafeCell).
    pub unsafe fn as_key_ref<'a>(self) -> &'a K
    where
        V: 'a,
    {
        // SAFETY: Safety requirements are covered by the containing function.
        let leaf = unsafe { self.as_ref() };

        leaf.key_ref()
    }

    /// Returns a unique mutable reference to the key and value of the pointed
    /// to [`LeafNode`].
    ///
    /// # Safety
    ///  - You must enforce Rust’s aliasing rules, since the returned lifetime
    ///    'a is arbitrarily chosen and does not necessarily reflect the actual
    ///    lifetime of the data. In particular, for the duration of this
    ///    lifetime, the memory the pointer points to must not get mutated
    ///    (except inside UnsafeCell).
    pub unsafe fn as_value_ref<'a>(self) -> &'a V
    where
        K: 'a,
        V: 'a,
    {
        // SAFETY: Safety requirements are covered by the containing function.
        let leaf = unsafe { self.as_ref() };

        leaf.value_ref()
    }

    /// Returns a unique mutable reference to the key and value of the pointed
    /// to [`LeafNode`].
    ///
    /// # Safety
    ///  - You must enforce Rust’s aliasing rules, since the returned lifetime
    ///    'a is arbitrarily chosen and does not necessarily reflect the actual
    ///    lifetime of the node. In particular, for the duration of this
    ///    lifetime, the node the pointer points to must not get accessed (read
    ///    or written) through any other pointer.
    pub unsafe fn as_value_mut<'a>(self) -> &'a mut V
    where
        K: 'a,
        V: 'a,
    {
        // SAFETY: Safety requirements are covered by the containing function.
        let leaf = unsafe { self.as_mut() };

        leaf.value_mut()
    }
}

impl<N: Node> Clone for NodePtr<N> {
    fn clone(&self) -> Self {
        Self(self.0)
    }
}
impl<N: Node> Copy for NodePtr<N> {}

impl<N: Node> From<&mut N> for NodePtr<N> {
    fn from(node_ref: &mut N) -> Self {
        // SAFETY: Pointer is non-null, aligned, and pointing to a valid instance of N
        // because it was constructed from a mutable reference.
        unsafe { NodePtr::new(node_ref as *mut _) }
    }
}

impl<N: Node> PartialEq for NodePtr<N> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<N: Node> Eq for NodePtr<N> {}

impl<N: Node> fmt::Debug for NodePtr<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("NodePtr").field(&self.0).finish()
    }
}

impl<N: Node> fmt::Pointer for NodePtr<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Pointer::fmt(&self.0, f)
    }
}

pub(crate) mod private {
    /// This trait is used to seal other traits, such that they cannot be
    /// implemented outside of the crate.
    pub trait Sealed {}

    impl<K, V> Sealed for super::InnerNode4<K, V> {}
    impl<K, V> Sealed for super::InnerNode16<K, V> {}
    impl<K, V> Sealed for super::InnerNode48<K, V> {}
    impl<K, V> Sealed for super::InnerNode256<K, V> {}
    impl<K, V> Sealed for super::LeafNode<K, V> {}
}

/// All nodes which contain a runtime tag that validates their type.
pub trait Node: private::Sealed {
    /// The runtime type of the node.
    const TYPE: NodeType;

    /// The key type carried by the leafe nodes
    type Key;

    /// The value type carried by the leaf nodes
    type Value;
}

/// Common methods implemented by all inner node.
pub trait InnerNode: Node {
    /// The type of the next larger node type.
    type GrownNode: InnerNode<Key = <Self as Node>::Key, Value = <Self as Node>::Value>;

    /// The type of the next smaller node type.
    type ShrunkNode: InnerNode<Key = <Self as Node>::Key, Value = <Self as Node>::Value>;

    /// The type of the iterator over all children of the inner node
    type Iter: Iterator<
            Item = (
                u8,
                OpaqueNodePtr<<Self as Node>::Key, <Self as Node>::Value>,
            ),
        > + DoubleEndedIterator
        + Into<InnerNodeIter<<Self as Node>::Key, <Self as Node>::Value>>;

    /// Search through this node for a child node that corresponds to the given
    /// key fragment.
    fn lookup_child(
        &self,
        key_fragment: u8,
    ) -> Option<OpaqueNodePtr<<Self as Node>::Key, <Self as Node>::Value>>;

    /// Write a child pointer with key fragment to this inner node.
    ///
    /// If the key fragment already exists in the node, overwrite the existing
    /// child pointer.
    ///
    /// # Panics
    ///
    /// Panics when the node is full.
    fn write_child(
        &mut self,
        key_fragment: u8,
        child_pointer: OpaqueNodePtr<<Self as Node>::Key, <Self as Node>::Value>,
    );

    /// Attempt to remove a child pointer at the key fragment from this inner
    /// node.
    ///
    /// If the key fragment does not exist in this node, return `None`.
    fn remove_child(
        &mut self,
        key_fragment: u8,
    ) -> Option<OpaqueNodePtr<<Self as Node>::Key, <Self as Node>::Value>>;

    /// Grow this node into the next larger class, copying over children and
    /// prefix information.
    fn grow(&self) -> Self::GrownNode;

    /// Shrink this node into the next smaller class, copying over children and
    /// prefix information.
    ///
    /// # Panics
    ///
    /// Panics if the new, smaller node size does not have enough capacity to
    /// hold all the children.
    fn shrink(&self) -> Self::ShrunkNode;

    /// Access the header information for this node.
    fn header(&self) -> &Header;

    /// Access the header information for this node.
    fn header_mut(&mut self) -> &mut Header;

    /// Returns true if this node has no more space to store children.
    fn is_full(&self) -> bool {
        self.header().num_children() >= Self::TYPE.upper_capacity()
    }

    /// Create an iterator over all (key bytes, child pointers) in this inner
    /// node.
    ///
    /// # Safety
    ///
    /// The iterator type does not carry any lifetime, so the caller of this
    /// function must enforce that the lifetime of the iterator does not overlap
    /// with any mutating operations on the node.
    unsafe fn iter(&self) -> Self::Iter;
}

enum WritePoint {
    Existing(usize),
    Last(usize),
    Shift(usize),
}

trait SearchInnerNodeCompressed {
    fn lookup_child_index(&self, key_fragment: u8) -> Option<usize>;
    fn find_write_point(&self, key_fragment: u8) -> WritePoint;
}

/// Node type that has a compact representation for key bytes and children
/// pointers.
#[repr(C, align(8))]
pub struct InnerNodeCompressed<K, V, const SIZE: usize> {
    /// The common node fields.
    pub header: Header,
    /// An array that contains single key bytes in the same index as the
    /// `child_pointers` array contains the matching child tree.
    ///
    /// This array will only be initialized for the first `header.num_children`
    /// values.
    pub keys: [MaybeUninit<u8>; SIZE],
    /// An array that contains the child data.
    ///
    /// This array will only be initialized for the first `header.num_children`
    /// values.
    pub child_pointers: [MaybeUninit<OpaqueNodePtr<K, V>>; SIZE],
}

impl<K, V, const SIZE: usize> Clone for InnerNodeCompressed<K, V, SIZE> {
    fn clone(&self) -> Self {
        Self {
            header: self.header.clone(),
            keys: self.keys,
            child_pointers: self.child_pointers,
        }
    }
}

impl<K, V, const SIZE: usize> fmt::Debug for InnerNodeCompressed<K, V, SIZE> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (keys, child_pointers) = self.initialized_portion();
        f.debug_struct("InnerNodeBlock")
            .field("SIZE", &SIZE)
            .field("header", &self.header)
            .field("keys", &keys)
            .field("child_pointers", &child_pointers)
            .finish()
    }
}

impl<K, V, const SIZE: usize> InnerNodeCompressed<K, V, SIZE> {
    /// Create an empty [`InnerNodeCompressed`] with the given [`NodeType`].
    pub fn empty() -> Self {
        InnerNodeCompressed {
            header: Header::default(),
            child_pointers: MaybeUninit::uninit_array(),
            keys: [MaybeUninit::new(0); SIZE],
        }
    }

    /// Return the initialized portions of the keys and child pointer arrays.
    pub fn initialized_portion(&self) -> (&[u8], &[OpaqueNodePtr<K, V>]) {
        // SAFETY: The array prefix with length `header.num_children` is guaranteed to
        // be initialized
        unsafe {
            let num_children = self.header.num_children();
            assume(num_children <= self.keys.len());
            (
                MaybeUninit::slice_assume_init_ref(self.keys.get_unchecked(0..num_children)),
                MaybeUninit::slice_assume_init_ref(
                    self.child_pointers.get_unchecked(0..num_children),
                ),
            )
        }
    }

    fn lookup_child_inner(&self, key_fragment: u8) -> Option<OpaqueNodePtr<K, V>>
    where
        Self: SearchInnerNodeCompressed,
    {
        let child_index = self.lookup_child_index(key_fragment)?;
        // SAFETY: The value at `child_index` is guaranteed to be initialized because
        // the `lookup_child_index` function will only search in the initialized portion
        // of the `child_pointers` array.
        Some(unsafe { MaybeUninit::assume_init(self.child_pointers[child_index]) })
    }

    fn write_child_inner(&mut self, key_fragment: u8, child_pointer: OpaqueNodePtr<K, V>)
    where
        Self: SearchInnerNodeCompressed,
    {
        let num_children = self.header.num_children();
        let child_index = match self.find_write_point(key_fragment) {
            WritePoint::Existing(child_index) => child_index,
            WritePoint::Last(child_index) => {
                self.header.num_children += 1;
                child_index
            },
            WritePoint::Shift(child_index) => {
                unsafe {
                    assume(num_children <= self.keys.len());
                    assume(child_index < num_children);
                }
                self.keys
                    .copy_within(child_index..num_children, child_index + 1);
                self.child_pointers
                    .copy_within(child_index..num_children, child_index + 1);
                self.header.num_children += 1;
                child_index
            },
        };
        unsafe {
            assume(child_index < self.keys.len());
            self.keys.get_unchecked_mut(child_index).write(key_fragment);
            self.child_pointers.get_unchecked_mut(child_index).write(child_pointer);
        }
    }

    /// Write a child without bounds check or order
    pub unsafe fn write_child_unchecked(&mut self, key_fragment: u8, child_pointer: OpaqueNodePtr<K, V>) {
        let i = self.header.num_children as usize;
        unsafe {
            assume(i < self.keys.len());
            self.keys.get_unchecked_mut(i).write(key_fragment);
            self.child_pointers.get_unchecked_mut(i).write(child_pointer);
        }
        self.header.num_children += 1;
    }

    fn remove_child_inner(&mut self, key_fragment: u8) -> Option<OpaqueNodePtr<K, V>>
    where
        Self: SearchInnerNodeCompressed,
    {
        let child_index = self.lookup_child_index(key_fragment)?;
        let child_ptr = mem::replace(&mut self.child_pointers[child_index], MaybeUninit::uninit());

        // Copy all the child_pointer and key values in higher indices down by one.
        self.child_pointers
            .copy_within((child_index + 1)..self.header.num_children(), child_index);
        self.keys
            .copy_within((child_index + 1)..self.header.num_children(), child_index);

        self.header.num_children -= 1;
        // SAFETY: This child pointer value is initialized because we got it by
        // searching through the initialized keys and got the `Ok(index)` value.
        Some(unsafe { MaybeUninit::assume_init(child_ptr) })
    }

    fn change_block_size<const NEW_SIZE: usize>(&self) -> InnerNodeCompressed<K, V, NEW_SIZE> {
        assert!(
            self.header.num_children() <= NEW_SIZE,
            "Cannot change InnerNodeCompressed<{}> to size {} when it has more than {} children. \
             Currently has [{}] children.",
            SIZE,
            NEW_SIZE,
            NEW_SIZE,
            self.header.num_children
        );

        let header = self.header.clone();
        let mut keys = [MaybeUninit::new(0); NEW_SIZE];
        let mut child_pointers = MaybeUninit::uninit_array();

        keys[..header.num_children()].copy_from_slice(&self.keys[..header.num_children()]);
        child_pointers[..header.num_children()]
            .copy_from_slice(&self.child_pointers[..header.num_children()]);

        InnerNodeCompressed {
            header,
            keys,
            child_pointers,
        }
    }

    fn grow_node48(&self) -> InnerNode48<K, V> {
        let header = self.header.clone();
        let mut child_indices = [RestrictedNodeIndex::<48>::EMPTY; 256];
        let mut child_pointers = MaybeUninit::uninit_array();

        let (n16_keys, _) = self.initialized_portion();

        assert_eq!(
            n16_keys.len(),
            self.keys.len(),
            "Node must be full to grow to node 48"
        );

        for (index, key) in n16_keys.iter().copied().enumerate() {
            // PANIC SAFETY: This `try_from` will not panic because index is guaranteed to
            // be 15 or less because of the length of the `InnerNode16.keys` array.
            child_indices[usize::from(key)] = RestrictedNodeIndex::try_from(index).unwrap();
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

/// Node that references between 2 and 4 children
pub type InnerNode4<K, V> = InnerNodeCompressed<K, V, 4>;

impl<K, V> SearchInnerNodeCompressed for InnerNode4<K, V> {
    fn lookup_child_index(&self, key_fragment: u8) -> Option<usize> {
        let (keys, _) = self.initialized_portion();
        for (child_index, key) in keys.iter().enumerate() {
            if key_fragment == *key {
                return Some(child_index);
            }
        }

        None
    }

    fn find_write_point(&self, key_fragment: u8) -> WritePoint {
        let (keys, _) = self.initialized_portion();

        let mut child_index = 0;
        for key in keys {
            if key_fragment < *key {
                return WritePoint::Shift(child_index);
            } else if key_fragment == *key {
                return WritePoint::Existing(child_index);
            }
            child_index += 1;
        }
        WritePoint::Last(child_index)
    }
}

impl<K, V> Node for InnerNode4<K, V> {
    type Key = K;
    type Value = V;

    const TYPE: NodeType = NodeType::Node4;
}

impl<K, V> InnerNode for InnerNode4<K, V> {
    type GrownNode = InnerNode16<<Self as Node>::Key, <Self as Node>::Value>;
    type Iter = InnerNodeCompressedIter<<Self as Node>::Key, <Self as Node>::Value>;
    type ShrunkNode = InnerNode4<<Self as Node>::Key, <Self as Node>::Value>;

    fn lookup_child(&self, key_fragment: u8) -> Option<OpaqueNodePtr<K, V>> {
        self.lookup_child_inner(key_fragment)
    }

    fn write_child(&mut self, key_fragment: u8, child_pointer: OpaqueNodePtr<K, V>) {
        self.write_child_inner(key_fragment, child_pointer)
    }

    fn remove_child(
        &mut self,
        key_fragment: u8,
    ) -> Option<OpaqueNodePtr<<Self as Node>::Key, <Self as Node>::Value>> {
        self.remove_child_inner(key_fragment)
    }

    fn grow(&self) -> Self::GrownNode {
        self.change_block_size()
    }

    fn shrink(&self) -> Self::ShrunkNode {
        panic!("unable to shrink a Node4, something went wrong!")
    }

    fn header(&self) -> &Header {
        &self.header
    }

    fn header_mut(&mut self) -> &mut Header {
        &mut self.header
    }

    unsafe fn iter(&self) -> Self::Iter {
        // SAFETY: The safety requirements on the `iter` function match the `new`
        // function
        unsafe { InnerNodeCompressedIter::new(self) }
    }
}

/// Node that references between 5 and 16 children
pub type InnerNode16<K, V> = InnerNodeCompressed<K, V, 16>;

impl<K, V> SearchInnerNodeCompressed for InnerNode16<K, V> {
    fn lookup_child_index(&self, key_fragment: u8) -> Option<usize> {
        const SIZE: usize = 16;
        type T = __m128i;
        unsafe {
            let keys = MaybeUninit::array_assume_init::<16>(self.keys);
            let cmp = T::from(
                Simd::<u8, SIZE>::splat(key_fragment)
                    .simd_eq(Simd::<u8, SIZE>::from_array(keys))
                    .to_int(),
            );
            let mask = (1i32 << self.header.num_children) - 1;
            let bitfield = _mm_movemask_epi8(cmp) & mask;
            if bitfield != 0 {
                Some(bitfield.trailing_zeros() as usize)
            } else {
                None
            }
        }
    }

    fn find_write_point(&self, key_fragment: u8) -> WritePoint {
        const SIZE: usize = 16;
        type T = __m128i;
        unsafe {
            match self.lookup_child_index(key_fragment) {
                Some(child_index) => WritePoint::Existing(child_index),
                None => {
                    let keys = MaybeUninit::array_assume_init::<16>(self.keys);
                    let cmp = T::from(
                        Simd::<u8, SIZE>::splat(key_fragment)
                            .simd_lt(Simd::<u8, SIZE>::from_array(keys))
                            .to_int(),
                    );
                    let mask = (1i32 << self.header.num_children) - 1;
                    let bitfield = _mm_movemask_epi8(cmp) & mask;
                    if bitfield != 0 {
                        WritePoint::Shift(bitfield.trailing_zeros() as usize)
                    } else {
                        WritePoint::Last(self.header.num_children())
                    }
                },
            }
        }
    }
}

impl<K, V> Node for InnerNode16<K, V> {
    type Key = K;
    type Value = V;

    const TYPE: NodeType = NodeType::Node16;
}

impl<K, V> InnerNode for InnerNode16<K, V> {
    type GrownNode = InnerNode48<<Self as Node>::Key, <Self as Node>::Value>;
    type Iter = InnerNodeCompressedIter<<Self as Node>::Key, <Self as Node>::Value>;
    type ShrunkNode = InnerNode4<<Self as Node>::Key, <Self as Node>::Value>;

    fn lookup_child(&self, key_fragment: u8) -> Option<OpaqueNodePtr<K, V>> {
        self.lookup_child_inner(key_fragment)
    }

    fn write_child(&mut self, key_fragment: u8, child_pointer: OpaqueNodePtr<K, V>) {
        self.write_child_inner(key_fragment, child_pointer)
    }

    fn remove_child(
        &mut self,
        key_fragment: u8,
    ) -> Option<OpaqueNodePtr<<Self as Node>::Key, <Self as Node>::Value>> {
        self.remove_child_inner(key_fragment)
    }

    fn grow(&self) -> Self::GrownNode {
        self.grow_node48()
    }

    fn shrink(&self) -> Self::ShrunkNode {
        self.change_block_size()
    }

    fn header(&self) -> &Header {
        &self.header
    }

    fn header_mut(&mut self) -> &mut Header {
        &mut self.header
    }

    unsafe fn iter(&self) -> Self::Iter {
        // SAFETY: The safety requirements on the `iter` function match the `new`
        // function
        unsafe { InnerNodeCompressedIter::new(self) }
    }
}

/// A restricted index only valid from 0 to LIMIT - 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct RestrictedNodeIndex<const LIMIT: u8>(u8);

impl<const LIMIT: u8> RestrictedNodeIndex<LIMIT> {
    /// A placeholder index value that indicates that the index is not occupied
    pub const EMPTY: Self = RestrictedNodeIndex(LIMIT);

    /// Return true if the given index is the empty sentinel value
    pub fn is_empty(self) -> bool {
        self == Self::EMPTY
    }
}

impl<const LIMIT: u8> From<RestrictedNodeIndex<LIMIT>> for u8 {
    fn from(src: RestrictedNodeIndex<LIMIT>) -> Self {
        src.0
    }
}

impl<const LIMIT: u8> From<RestrictedNodeIndex<LIMIT>> for usize {
    fn from(src: RestrictedNodeIndex<LIMIT>) -> Self {
        usize::from(src.0)
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

impl<const LIMIT: u8> TryFrom<u8> for RestrictedNodeIndex<LIMIT> {
    type Error = TryFromByteError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if value < LIMIT {
            Ok(RestrictedNodeIndex(value))
        } else {
            Err(TryFromByteError(LIMIT, usize::from(value)))
        }
    }
}

impl<const LIMIT: u8> PartialOrd for RestrictedNodeIndex<LIMIT> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        if self.0 == LIMIT || other.0 == LIMIT {
            None
        } else {
            Some(self.0.cmp(&other.0))
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
#[repr(C, align(8))]
pub struct InnerNode48<K, V> {
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
    pub child_pointers: [MaybeUninit<OpaqueNodePtr<K, V>>; 48],
}

impl<K, V> fmt::Debug for InnerNode48<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InnerNode48")
            .field("header", &self.header)
            .field("child_indices", &self.child_indices)
            .field("child_pointers", &self.child_pointers)
            .finish()
    }
}

impl<K, V> Clone for InnerNode48<K, V> {
    fn clone(&self) -> Self {
        Self {
            header: self.header.clone(),
            child_indices: self.child_indices,
            child_pointers: self.child_pointers,
        }
    }
}

impl<K, V> InnerNode48<K, V> {
    /// Create an empty `Node48`.
    pub fn empty() -> Self {
        InnerNode48 {
            header: Header::default(),
            child_indices: [RestrictedNodeIndex::<48>::EMPTY; 256],
            child_pointers: MaybeUninit::uninit_array(),
        }
    }

    /// Return the initialized portions of the child pointer array.
    pub fn initialized_child_pointers(&self) -> &[OpaqueNodePtr<K, V>] {
        // SAFETY: The array prefix with length `header.num_children` is guaranteed to
        // be initialized
        unsafe {
            MaybeUninit::slice_assume_init_ref(&self.child_pointers[0..self.header.num_children()])
        }
    }
}

impl<K, V> Node for InnerNode48<K, V> {
    type Key = K;
    type Value = V;

    const TYPE: NodeType = NodeType::Node48;
}

impl<K, V> InnerNode for InnerNode48<K, V> {
    type GrownNode = InnerNode256<<Self as Node>::Key, <Self as Node>::Value>;
    type Iter = InnerNode48Iter<<Self as Node>::Key, <Self as Node>::Value>;
    type ShrunkNode = InnerNode16<<Self as Node>::Key, <Self as Node>::Value>;

    fn lookup_child(
        &self,
        key_fragment: u8,
    ) -> Option<OpaqueNodePtr<<Self as Node>::Key, <Self as Node>::Value>> {
        let index = &self.child_indices[usize::from(key_fragment)];
        let child_pointers = self.initialized_child_pointers();
        if !index.is_empty() {
            Some(child_pointers[usize::from(*index)])
        } else {
            None
        }
    }

    fn write_child(
        &mut self,
        key_fragment: u8,
        child_pointer: OpaqueNodePtr<<Self as Node>::Key, <Self as Node>::Value>,
    ) {
        let key_fragment_idx = usize::from(key_fragment);
        let child_index = if self.child_indices[key_fragment_idx] == RestrictedNodeIndex::EMPTY {
            let child_index = self.header.num_children();
            assert!(child_index < self.child_pointers.len(), "node is full");

            self.child_indices[key_fragment_idx] =
                RestrictedNodeIndex::try_from(child_index).unwrap();
            self.header.num_children += 1;
            child_index
        } else {
            // overwrite existing
            usize::from(self.child_indices[key_fragment_idx])
        };
        self.child_pointers[child_index].write(child_pointer);
    }

    fn remove_child(
        &mut self,
        key_fragment: u8,
    ) -> Option<OpaqueNodePtr<<Self as Node>::Key, <Self as Node>::Value>> {
        let restricted_index = self.child_indices[usize::from(key_fragment)];
        if restricted_index.is_empty() {
            return None;
        }

        // Replace child pointer with unitialized value, even though it may possibly be
        // overwritten by the compaction step
        let child_ptr = mem::replace(
            &mut self.child_pointers[usize::from(restricted_index)],
            MaybeUninit::uninit(),
        );

        // Copy all the child_pointer values in higher indices down by one.
        self.child_pointers.copy_within(
            (usize::from(restricted_index) + 1)..self.header.num_children(),
            usize::from(restricted_index),
        );

        // Take all child indices that are greater than the index we're removing, and
        // subtract one so that they remain valid
        for other_restrict_index in &mut self.child_indices {
            if matches!(
                restricted_index.partial_cmp(other_restrict_index),
                Some(Ordering::Less)
            ) {
                // PANIC SAFETY: This will not underflow, because it is guaranteed to be
                // greater than at least 1 other index. This will not panic in the
                // `try_from` because the new value is derived from an existing restricted
                // index.
                *other_restrict_index =
                    RestrictedNodeIndex::try_from(other_restrict_index.0 - 1).unwrap();
            }
        }

        self.child_indices[usize::from(key_fragment)] = RestrictedNodeIndex::EMPTY;
        self.header.num_children -= 1;
        // SAFETY: This child pointer value is initialized because we got it by using a
        // non-`RestrictedNodeIndex::<>::EMPTY` index from the child indices array.
        Some(unsafe { MaybeUninit::assume_init(child_ptr) })
    }

    fn grow(&self) -> Self::GrownNode {
        let header = self.header.clone();
        let mut child_pointers = [None; 256];
        // SAFETY: This iterator lives only for the lifetime of this function, which
        // does not mutate the `InnerNode48` (guaranteed by reference).
        let iter = unsafe { InnerNode48Iter::new(self) };

        for (key_fragment, child_pointer) in iter {
            child_pointers[usize::from(key_fragment)] = Some(child_pointer);
        }

        InnerNode256 {
            header,
            child_pointers,
        }
    }

    fn shrink(&self) -> Self::ShrunkNode {
        assert!(
            self.header.num_children <= 16,
            "Cannot shrink a Node48 when it has more than 16 children. Currently has [{}] \
             children.",
            self.header.num_children
        );

        let header = self.header.clone();

        let mut key_and_child_ptrs = MaybeUninit::uninit_array::<16>();
        // SAFETY: The lifetime of this iterator is bounded to this function, and will
        // not overlap with any mutating operations on the node because it of the shared
        // reference that this function uses.
        let iter = unsafe { InnerNode48Iter::new(self) };

        for (idx, value) in iter.enumerate() {
            key_and_child_ptrs[idx].write(value);
        }

        let init_key_and_child_ptrs = {
            // SAFETY: The first `num_children` are guaranteed to be initialized in this
            // array because the previous iterator loops through all children of the inner
            // node.
            let init_key_and_child_ptrs = unsafe {
                MaybeUninit::slice_assume_init_mut(&mut key_and_child_ptrs[..header.num_children()])
            };

            init_key_and_child_ptrs.sort_unstable_by_key(|(key_byte, _)| *key_byte);

            init_key_and_child_ptrs
        };

        let mut keys = MaybeUninit::uninit_array();
        let mut child_pointers = MaybeUninit::uninit_array();

        for (idx, (key_byte, child_ptr)) in init_key_and_child_ptrs.iter().copied().enumerate() {
            keys[idx].write(key_byte);
            child_pointers[idx].write(child_ptr);
        }

        InnerNodeCompressed {
            header,
            keys,
            child_pointers,
        }
    }

    fn header(&self) -> &Header {
        &self.header
    }

    fn header_mut(&mut self) -> &mut Header {
        &mut self.header
    }

    unsafe fn iter(&self) -> Self::Iter {
        // SAFETY: The safety requirements on the `iter` function match the `new`
        // function
        unsafe { InnerNode48Iter::new(self) }
    }
}

/// Node that references between 49 and 256 children
#[repr(C, align(8))]
pub struct InnerNode256<K, V> {
    /// The common node fields.
    pub header: Header,
    /// An array that directly maps a key byte (as index) to a child node.
    pub child_pointers: [Option<OpaqueNodePtr<K, V>>; 256],
}

impl<K, V> fmt::Debug for InnerNode256<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InnerNode256")
            .field("header", &self.header)
            .field("child_pointers", &self.child_pointers)
            .finish()
    }
}

impl<K, V> Clone for InnerNode256<K, V> {
    fn clone(&self) -> Self {
        Self {
            header: self.header.clone(),
            child_pointers: self.child_pointers,
        }
    }
}

impl<K, V> InnerNode256<K, V> {
    /// Create an empty `InnerNode256`.
    pub fn empty() -> Self {
        InnerNode256 {
            header: Header::default(),
            child_pointers: [None; 256],
        }
    }
}

impl<K, V> Node for InnerNode256<K, V> {
    type Key = K;
    type Value = V;

    const TYPE: NodeType = NodeType::Node256;
}

impl<K, V> InnerNode for InnerNode256<K, V> {
    type GrownNode = Self;
    type Iter = InnerNode256Iter<K, V>;
    type ShrunkNode = InnerNode48<K, V>;

    fn lookup_child(
        &self,
        key_fragment: u8,
    ) -> Option<OpaqueNodePtr<<Self as Node>::Key, <Self as Node>::Value>> {
        self.child_pointers[usize::from(key_fragment)]
    }

    fn write_child(
        &mut self,
        key_fragment: u8,
        child_pointer: OpaqueNodePtr<<Self as Node>::Key, <Self as Node>::Value>,
    ) {
        let key_fragment_idx = usize::from(key_fragment);
        let existing_pointer = self.child_pointers[key_fragment_idx];
        self.child_pointers[key_fragment_idx] = Some(child_pointer);
        if existing_pointer.is_none() {
            self.header.num_children += 1;
        }
    }

    fn remove_child(
        &mut self,
        key_fragment: u8,
    ) -> Option<OpaqueNodePtr<<Self as Node>::Key, <Self as Node>::Value>> {
        let removed_child = self.child_pointers[usize::from(key_fragment)].take();

        if removed_child.is_some() {
            self.header.num_children -= 1;
        }

        removed_child
    }

    fn grow(&self) -> Self::GrownNode {
        panic!("unable to grow a Node256, something went wrong!")
    }

    fn shrink(&self) -> Self::ShrunkNode {
        assert!(
            self.header.num_children <= 48,
            "Cannot shrink a Node256 when it has more than 48 children. Currently has [{}] \
             children.",
            self.header.num_children
        );

        let header = self.header.clone();
        let mut child_indices = [RestrictedNodeIndex::<48>::EMPTY; 256];
        let mut child_pointers = MaybeUninit::uninit_array();

        // SAFETY: This iterator lives only for the lifetime of this function, which
        // does not mutate the `InnerNode256` (guaranteed by reference).
        let iter = unsafe { InnerNode256Iter::new(self) };

        for (child_index, (key_byte, child_ptr)) in iter.enumerate() {
            // PANIC SAFETY: This `try_from` will not panic because the `next_index` value
            // is guaranteed to be 48 or less by the `assert!(num_children < 48)` at the
            // start of the function.
            child_indices[usize::from(key_byte)] =
                unsafe { RestrictedNodeIndex::try_from(child_index).unwrap() };
            child_pointers[child_index].write(child_ptr);
        }

        InnerNode48 {
            header,
            child_indices,
            child_pointers,
        }
    }

    fn header(&self) -> &Header {
        &self.header
    }

    fn header_mut(&mut self) -> &mut Header {
        &mut self.header
    }

    unsafe fn iter(&self) -> Self::Iter {
        // SAFETY: The safety requirements on the `iter` function match the `new`
        // function
        unsafe { InnerNode256Iter::new(self) }
    }
}

/// Node that contains a single leaf value.
#[derive(Debug, Clone)]
#[repr(align(8))]
pub struct LeafNode<K, V> {
    /// The leaf value.
    value: V,
    /// The full key that the `value` was stored with.
    key: K,
}

impl<K, V> LeafNode<K, V> {
    /// Create a new leaf node with the given value.
    pub fn new(key: K, value: V) -> Self {
        LeafNode { value, key }
    }

    /// Returns a shared reference to the key contained by this leaf node
    pub fn key_ref(&self) -> &K {
        &self.key
    }

    /// Returns a shared reference to the value contained by this leaf node
    pub fn value_ref(&self) -> &V {
        &self.value
    }

    /// Returns a mutable reference to the value contained by this leaf node
    pub fn value_mut(&mut self) -> &mut V {
        &mut self.value
    }

    /// Return shared references to the key and value contained by this leaf
    /// node
    pub fn entry_ref(&self) -> (&K, &V) {
        (&self.key, &self.value)
    }

    /// Return mutable references to the key and value contained by this leaf
    /// node
    pub fn entry_mut(&mut self) -> (&mut K, &mut V) {
        (&mut self.key, &mut self.value)
    }

    /// Consume the leaf node and return a tuple of the key and value
    pub fn into_entry(self) -> (K, V) {
        (self.key, self.value)
    }

    /// Check that the provided full key is the same one as the stored key.
    pub fn matches_full_key<Q>(&self, possible_key: &Q) -> bool
    where
        K: Borrow<Q> + AsBytes,
        Q: AsBytes + ?Sized,
    {
        self.key.borrow().as_bytes().eq(possible_key.as_bytes())
    }
}

impl<K, V> Node for LeafNode<K, V> {
    type Key = K;
    type Value = V;

    const TYPE: NodeType = NodeType::Leaf;
}
