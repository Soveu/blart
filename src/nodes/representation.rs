//! Trie node representation

use crate::{rust_nightly_apis::assume, tagged_pointer::TaggedPointer, AsBytes, Header};
use std::{
    borrow::Borrow,
    fmt,
    hash::Hash,
    iter::FusedIterator,
    marker::PhantomData,
    mem::{self, ManuallyDrop},
    ops::Range,
    ptr::{self, NonNull},
};

mod inner_node_256;
pub use inner_node_256::*;

mod inner_node_48;
pub use inner_node_48::*;

mod inner_node_compressed;
pub use inner_node_compressed::*;

#[cfg(test)]
mod tests;

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

    /// Converts a u8 value to a [`NodeType`]
    ///
    /// # Safety
    ///  - `src` must be a valid variant from the enum
    pub const unsafe fn from_u8(src: u8) -> NodeType {
        // SAFETY: `NodeType` is repr(u8)
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
pub struct OpaqueNodePtr<K: AsBytes, V, const PREFIX_LEN: usize>(
    TaggedPointer<OpaqueValue, 3>,
    PhantomData<(K, V)>,
);

impl<K: AsBytes, V, const PREFIX_LEN: usize> Copy for OpaqueNodePtr<K, V, PREFIX_LEN> {}

impl<K: AsBytes, V, const PREFIX_LEN: usize> Clone for OpaqueNodePtr<K, V, PREFIX_LEN> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K: AsBytes, V, const PREFIX_LEN: usize> fmt::Debug for OpaqueNodePtr<K, V, PREFIX_LEN> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("OpaqueNodePtr").field(&self.0).finish()
    }
}

impl<K: AsBytes, V, const PREFIX_LEN: usize> fmt::Pointer for OpaqueNodePtr<K, V, PREFIX_LEN> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Pointer::fmt(&self.0, f)
    }
}

impl<K: AsBytes, V, const PREFIX_LEN: usize> Eq for OpaqueNodePtr<K, V, PREFIX_LEN> {}

impl<K: AsBytes, V, const PREFIX_LEN: usize> PartialEq for OpaqueNodePtr<K, V, PREFIX_LEN> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<K: AsBytes, V, const PREFIX_LEN: usize> Hash for OpaqueNodePtr<K, V, PREFIX_LEN> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl<K: AsBytes, V, const PREFIX_LEN: usize> OpaqueNodePtr<K, V, PREFIX_LEN> {
    /// Construct a new opaque node pointer from an existing non-null node
    /// pointer.
    pub fn new<N>(pointer: NonNull<N>) -> Self
    where
        N: Node<PREFIX_LEN, Value = V>,
    {
        let mut tagged_ptr = TaggedPointer::from(pointer).cast::<OpaqueValue>();
        tagged_ptr.set_data(N::TYPE as usize);

        OpaqueNodePtr(tagged_ptr, PhantomData)
    }

    /// Return `true` if this Node_ pointer points to the specified concrete
    /// [`NodeType`].
    pub fn is<N: Node<PREFIX_LEN>>(&self) -> bool {
        self.0.to_data() == usize::from(N::TYPE as u8)
    }

    /// Create a non-opaque node pointer that will eliminate future type
    /// assertions, if the type of the pointed node matches the given
    /// node type.
    pub fn cast<N: Node<PREFIX_LEN>>(self) -> Option<NodePtr<PREFIX_LEN, N>> {
        if self.is::<N>() {
            Some(NodePtr(self.0.cast::<N>().into()))
        } else {
            None
        }
    }

    /// Cast this opaque pointer type an enum that contains a pointer to the
    /// concrete node type.
    pub fn to_node_ptr(self) -> ConcreteNodePtr<K, V, PREFIX_LEN> {
        match self.node_type() {
            NodeType::Node4 => ConcreteNodePtr::Node4(NodePtr(
                self.0.cast::<InnerNode4<K, V, PREFIX_LEN>>().into(),
            )),
            NodeType::Node16 => ConcreteNodePtr::Node16(NodePtr(
                self.0.cast::<InnerNode16<K, V, PREFIX_LEN>>().into(),
            )),
            NodeType::Node48 => ConcreteNodePtr::Node48(NodePtr(
                self.0.cast::<InnerNode48<K, V, PREFIX_LEN>>().into(),
            )),
            NodeType::Node256 => ConcreteNodePtr::Node256(NodePtr(
                self.0.cast::<InnerNode256<K, V, PREFIX_LEN>>().into(),
            )),
            NodeType::Leaf => ConcreteNodePtr::LeafNode(NodePtr(
                self.0.cast::<LeafNode<K, V, PREFIX_LEN>>().into(),
            )),
        }
    }

    /// Retrieve the runtime node type information.
    pub fn node_type(self) -> NodeType {
        // SAFETY: We know that we can convert the usize into a `NodeType` because
        // we have only stored `NodeType` values into this pointer
        unsafe { NodeType::from_u8(self.0.to_data() as u8) }
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
    pub(crate) unsafe fn header_mut<'h>(self) -> Option<&'h mut Header<PREFIX_LEN>> {
        let header_ptr = match self.node_type() {
            NodeType::Node4 | NodeType::Node16 | NodeType::Node48 | NodeType::Node256 => unsafe {
                self.header_mut_unchecked()
            },
            NodeType::Leaf => {
                return None;
            },
        };

        // SAFETY: The pointer is properly aligned and points to a initialized instance
        // of Header that is dereferenceable. The lifetime safety requirements are
        // passed up to the caller of this function.
        Some(header_ptr)
    }

    /// Get a mutable reference to the header, this doesn't check if the pointer
    /// is to an inner node.
    ///
    /// # Safety
    ///  - The pointer must be to an inner node
    ///  - You must enforce Rust’s aliasing rules, since the returned lifetime
    ///    'h is arbitrarily chosen and does not necessarily reflect the actual
    ///    lifetime of the data. In particular, for the duration of this
    ///    lifetime, the memory the pointer points to must not get accessed
    ///    (read or written) through any other pointer.
    pub(crate) unsafe fn header_mut_unchecked<'h>(self) -> &'h mut Header<PREFIX_LEN> {
        unsafe { &mut *self.0.cast::<Header<PREFIX_LEN>>().to_ptr() }
    }

    /// Do a deep clone recursively, by allocating new nodes
    pub fn deep_clone(&self) -> Self
    where
        K: Clone,
        V: Clone,
    {
        // SAFETY: We hold a shared reference, so it's safe to make
        // a shared reference from it
        match self.to_node_ptr() {
            ConcreteNodePtr::Node4(inner) => unsafe { inner.as_ref().deep_clone().to_opaque() },
            ConcreteNodePtr::Node16(inner) => unsafe { inner.as_ref().deep_clone().to_opaque() },
            ConcreteNodePtr::Node48(inner) => unsafe { inner.as_ref().deep_clone().to_opaque() },
            ConcreteNodePtr::Node256(inner) => unsafe { inner.as_ref().deep_clone().to_opaque() },
            ConcreteNodePtr::LeafNode(inner) => unsafe {
                NodePtr::allocate_node_ptr(inner.as_ref().clone()).to_opaque()
            },
        }
    }
}

/// An enum that encapsulates pointers to every type of Node
pub enum ConcreteNodePtr<K: AsBytes, V, const PREFIX_LEN: usize> {
    /// Node that references between 2 and 4 children
    Node4(NodePtr<PREFIX_LEN, InnerNode4<K, V, PREFIX_LEN>>),
    /// Node that references between 5 and 16 children
    Node16(NodePtr<PREFIX_LEN, InnerNode16<K, V, PREFIX_LEN>>),
    /// Node that references between 17 and 49 children
    Node48(NodePtr<PREFIX_LEN, InnerNode48<K, V, PREFIX_LEN>>),
    /// Node that references between 49 and 256 children
    Node256(NodePtr<PREFIX_LEN, InnerNode256<K, V, PREFIX_LEN>>),
    /// Node that contains a single value
    LeafNode(NodePtr<PREFIX_LEN, LeafNode<K, V, PREFIX_LEN>>),
}

impl<K: AsBytes, V, const PREFIX_LEN: usize> fmt::Debug for ConcreteNodePtr<K, V, PREFIX_LEN> {
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
pub struct NodePtr<const PREFIX_LEN: usize, N: Node<PREFIX_LEN>>(NonNull<N>);

impl<const PREFIX_LEN: usize, N: Node<PREFIX_LEN>> NodePtr<PREFIX_LEN, N> {
    /// Create a safe pointer to a [`Node`].
    ///
    /// # Safety
    /// - Given pointer must be non-null, aligned, and valid for reads or writes
    ///   of a value of N type.
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
    ///  - This function can only be called once for a given node object.
    #[must_use]
    pub unsafe fn deallocate_node_ptr(node: Self) -> N {
        // SAFETY: Covered by safety condition on function
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
    pub fn to_opaque(self) -> OpaqueNodePtr<N::Key, N::Value, PREFIX_LEN> {
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

    fn as_mut_safe(&mut self) -> &mut N {
        // SAFETY: The pointer is properly aligned and points to a initialized instance
        // of N that is dereferenceable. The lifetime safety requirements are passed up
        // to the invoked of this function.
        unsafe { self.0.as_mut() }
    }
}

impl<K: AsBytes, V, const PREFIX_LEN: usize> NodePtr<PREFIX_LEN, LeafNode<K, V, PREFIX_LEN>> {
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

impl<const PREFIX_LEN: usize, N: Node<PREFIX_LEN>> Clone for NodePtr<PREFIX_LEN, N> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<const PREFIX_LEN: usize, N: Node<PREFIX_LEN>> Copy for NodePtr<PREFIX_LEN, N> {}

impl<const PREFIX_LEN: usize, N: Node<PREFIX_LEN>> From<&mut N> for NodePtr<PREFIX_LEN, N> {
    fn from(node_ref: &mut N) -> Self {
        // SAFETY: Pointer is non-null, aligned, and pointing to a valid instance of N
        // because it was constructed from a mutable reference.
        unsafe { NodePtr::new(node_ref as *mut _) }
    }
}

impl<const PREFIX_LEN: usize, N: Node<PREFIX_LEN>> PartialEq for NodePtr<PREFIX_LEN, N> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<const PREFIX_LEN: usize, N: Node<PREFIX_LEN>> Eq for NodePtr<PREFIX_LEN, N> {}

impl<const PREFIX_LEN: usize, N: Node<PREFIX_LEN>> fmt::Debug for NodePtr<PREFIX_LEN, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("NodePtr").field(&self.0).finish()
    }
}

impl<const PREFIX_LEN: usize, N: Node<PREFIX_LEN>> fmt::Pointer for NodePtr<PREFIX_LEN, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Pointer::fmt(&self.0, f)
    }
}

pub(crate) mod private {
    use crate::AsBytes;

    /// This trait is used to seal other traits, such that they cannot be
    /// implemented outside of the crate.
    pub trait Sealed {}

    impl<K: AsBytes, V, const PREFIX_LEN: usize> Sealed for super::InnerNode4<K, V, PREFIX_LEN> {}
    impl<K: AsBytes, V, const PREFIX_LEN: usize> Sealed for super::InnerNode16<K, V, PREFIX_LEN> {}
    impl<K: AsBytes, V, const PREFIX_LEN: usize> Sealed for super::InnerNode48<K, V, PREFIX_LEN> {}
    impl<K: AsBytes, V, const PREFIX_LEN: usize> Sealed for super::InnerNode256<K, V, PREFIX_LEN> {}
    impl<K: AsBytes, V, const PREFIX_LEN: usize> Sealed for super::LeafNode<K, V, PREFIX_LEN> {}
}

/// All nodes which contain a runtime tag that validates their type.
pub trait Node<const PREFIX_LEN: usize>: private::Sealed {
    /// The runtime type of the node.
    const TYPE: NodeType;

    /// The key type carried by the leaf nodes
    type Key: AsBytes;

    /// The value type carried by the leaf nodes
    type Value;
}

/// Result of prefix match
#[derive(Debug)]
pub enum MatchPrefixResult<K: AsBytes, V, const PREFIX_LEN: usize> {
    /// If prefixes don't match
    Mismatch {
        /// Mismatch object
        mismatch: Mismatch<K, V, PREFIX_LEN>,
    },
    /// If the prefixes match entirely
    Match {
        /// How many bytes were matched
        matched_bytes: usize,
    },
}

/// Represents a prefix mismatch
#[derive(Debug)]
pub struct Mismatch<K: AsBytes, V, const PREFIX_LEN: usize> {
    /// How many bytes were matched
    pub matched_bytes: usize,
    /// Value of the byte that made it not match
    pub prefix_byte: u8,
    /// Pointer to the leaf if the prefix was reconstructed
    pub leaf_ptr: Option<NodePtr<PREFIX_LEN, LeafNode<K, V, PREFIX_LEN>>>,
}

/// Common methods implemented by all inner node.
pub trait InnerNode<const PREFIX_LEN: usize>: Node<PREFIX_LEN> + Sized {
    /// The type of the next larger node type.
    type GrownNode: InnerNode<PREFIX_LEN, Key = Self::Key, Value = Self::Value>;

    /// The type of the next smaller node type.
    type ShrunkNode: InnerNode<PREFIX_LEN, Key = Self::Key, Value = Self::Value>;

    /// The type of the iterator over all children of the inner node
    type Iter<'a>: Iterator<Item = (u8, OpaqueNodePtr<Self::Key, Self::Value, PREFIX_LEN>)>
        + DoubleEndedIterator
        + FusedIterator
    where
        Self: 'a;

    /// Create an empty [`InnerNode`], with no children and no prefix
    fn empty() -> Self {
        Self::from_header(Header::empty())
    }

    /// Create a new [`InnerNode`] using
    /// `prefix` as the node prefix and
    /// `prefix_len` as the node prefix length and
    ///
    /// This is done because when a prefix mismatch happens
    /// the length of the mismatch can be grater or equal to
    /// prefix size, since we search for the first child of the
    /// node to recreate the prefix, that's why we don't use
    /// `prefix.len()` as the node prefix length
    fn from_prefix(prefix: &[u8], prefix_len: usize) -> Self {
        Self::from_header(Header::new(prefix, prefix_len))
    }

    /// Create a new [`InnerNode`] using a `Header`
    fn from_header(header: Header<PREFIX_LEN>) -> Self;

    /// Get the `Header` from the [`InnerNode`]
    fn header(&self) -> &Header<PREFIX_LEN>;

    /// Search through this node for a child node that corresponds to the given
    /// key fragment.
    fn lookup_child(
        &self,
        key_fragment: u8,
    ) -> Option<OpaqueNodePtr<Self::Key, Self::Value, PREFIX_LEN>>;

    /// Write a child pointer with key fragment to this inner node.
    ///
    /// If the key fragment already exists in the node, overwrite the existing
    /// child pointer.
    ///
    /// # Panics
    ///  - Panics when the node is full.
    fn write_child(
        &mut self,
        key_fragment: u8,
        child_pointer: OpaqueNodePtr<Self::Key, Self::Value, PREFIX_LEN>,
    );

    /// Attempt to remove a child pointer at the key fragment from this inner
    /// node.
    ///
    /// If the key fragment does not exist in this node, return `None`.
    fn remove_child(
        &mut self,
        key_fragment: u8,
    ) -> Option<OpaqueNodePtr<Self::Key, Self::Value, PREFIX_LEN>>;

    /// Grow this node into the next larger class, copying over children and
    /// prefix information.
    fn grow(&self) -> Self::GrownNode;

    /// Shrink this node into the next smaller class, copying over children and
    /// prefix information.
    ///
    /// # Panics
    ///  - Panics if the new, smaller node size does not have enough capacity to
    ///    hold all the children.
    fn shrink(&self) -> Self::ShrunkNode;

    /// Returns true if this node has no more space to store children.
    fn is_full(&self) -> bool {
        self.header().num_children() >= Self::TYPE.upper_capacity()
    }

    /// Create an iterator over all (key bytes, child pointers) in this inner
    /// node.
    fn iter(&self) -> Self::Iter<'_>;

    /// Compares the compressed path of a node with the key and returns the
    /// number of equal bytes.
    ///
    /// # Safety
    ///  - `current_depth` > key len
    #[inline(always)]
    fn match_prefix(
        &self,
        key: &[u8],
        current_depth: usize,
    ) -> MatchPrefixResult<Self::Key, Self::Value, PREFIX_LEN> {
        #[allow(unused_unsafe)]
        unsafe {
            // SAFETY: Since we are iterating the key and prefixes, we
            // expect that the depth never exceeds the key len.
            // Because if this happens we ran out of bytes in the key to match
            // and the whole process should be already finished
            assume!(current_depth <= key.len());
        }
        let (prefix, leaf_ptr) = self.read_full_prefix(current_depth);
        let key = &key[current_depth..];

        let matched_bytes = prefix
            .iter()
            .zip(key)
            .take_while(|(a, b)| **a == **b)
            .count();
        if matched_bytes < prefix.len() {
            MatchPrefixResult::Mismatch {
                mismatch: Mismatch {
                    matched_bytes,
                    prefix_byte: prefix[matched_bytes],
                    leaf_ptr,
                },
            }
        } else {
            MatchPrefixResult::Match { matched_bytes }
        }
    }

    /// Read the prefix as a whole, by reconstructing it if necessary from a
    /// leaf
    #[inline(always)]
    fn read_full_prefix(
        &self,
        current_depth: usize,
    ) -> (
        &[u8],
        Option<NodePtr<PREFIX_LEN, LeafNode<Self::Key, Self::Value, PREFIX_LEN>>>,
    ) {
        self.header().inner_read_full_prefix(self, current_depth)
    }

    /// Returns the minimum child pointer from this node and it's key
    ///
    /// # Safety
    ///  - Since this is a [`InnerNode`] we assume that the we have at least one
    ///    child, (more strictly we have 2, because with one child the node
    ///    would have collapsed) so in this way we can avoid the [`Option`].
    ///    This is safe because if we had no children this current node should
    ///    have been deleted.
    fn min(&self) -> (u8, OpaqueNodePtr<Self::Key, Self::Value, PREFIX_LEN>);

    /// Returns the maximum child pointer from this node and it's key
    ///
    /// # Safety
    ///  - Since this is a [`InnerNode`] we assume that the we have at least one
    ///    child, (more strictly we have 2, because with one child the node
    ///    would have collapsed) so in this way we can avoid the [`Option`].
    ///    This is safe because if we had, no children this current node should
    ///    have been deleted.
    fn max(&self) -> (u8, OpaqueNodePtr<Self::Key, Self::Value, PREFIX_LEN>);

    /// Deep clones the inner node by allocating memory to a new one
    fn deep_clone(&self) -> NodePtr<PREFIX_LEN, Self>
    where
        Self::Key: Clone,
        Self::Value: Clone;
}

/// Node that contains a single leaf value.
#[derive(Debug, Clone)]
#[repr(align(8))]
pub struct LeafNode<K, V, const PREFIX_LEN: usize> {
    /// The leaf value.
    value: V,
    /// The full key that the `value` was stored with.
    key: K,
}

impl<K, V, const PREFIX_LEN: usize> LeafNode<K, V, PREFIX_LEN> {
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

impl<K: AsBytes, V, const PREFIX_LEN: usize> Node<PREFIX_LEN> for LeafNode<K, V, PREFIX_LEN> {
    type Key = K;
    type Value = V;

    const TYPE: NodeType = NodeType::Leaf;
}
