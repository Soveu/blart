use crate::nodes::NodePtr;

use super::*;

#[test]
fn lookup_on_leaf() {
    let mut leaf: LeafNode<i32> = LeafNode::new(vec![1, 2, 3], 123);
    let leaf_ptr = NodePtr::from(&mut leaf).to_opaque();

    // SAFETY: The type parameter (`i32`) matches the type that the leaf was
    // constructed with.
    unsafe {
        assert_eq!(search::<i32>(leaf_ptr, &[1, 2, 3]), Some(&123));
        assert!(ptr::eq(
            search::<i32>(leaf_ptr, &[1, 2, 3]).unwrap(),
            &leaf.value
        ));
        assert_eq!(search::<i32>(leaf_ptr, &[0, 0, 0]), None)
    }
}

#[test]
fn lookup_on_full_node4() {
    let mut l1: LeafNode<i32> = LeafNode::new(vec![1, 2, 1], 121);
    let mut l2: LeafNode<i32> = LeafNode::new(vec![1, 2, 2], 122);
    let mut l3: LeafNode<i32> = LeafNode::new(vec![1, 2, 3], 123);
    let mut l4: LeafNode<i32> = LeafNode::new(vec![1, 2, 4], 124);

    let l1_ptr = NodePtr::from(&mut l1).to_opaque();
    let l2_ptr = NodePtr::from(&mut l2).to_opaque();
    let l3_ptr = NodePtr::from(&mut l3).to_opaque();
    let l4_ptr = NodePtr::from(&mut l4).to_opaque();

    let mut inner_node = InnerNode4::empty();

    // Update inner node prefix and child slots
    inner_node.header.write_prefix(&[1, 2]);
    inner_node.write_child(1, l1_ptr);
    inner_node.write_child(2, l2_ptr);
    inner_node.write_child(3, l3_ptr);
    inner_node.write_child(4, l4_ptr);

    let root = NodePtr::from(&mut inner_node).to_opaque();

    // SAFETY: All the `search` calls are safe because they are specifing the `i32`
    // output type parameter, the same one that was used to construct the leaf
    // nodes.
    unsafe {
        assert_eq!(search::<i32>(root, &[1, 2, 1]), Some(&121));
        assert_eq!(search::<i32>(root, &[1, 2, 2]), Some(&122));
        assert_eq!(search::<i32>(root, &[1, 2, 3]), Some(&123));
        assert_eq!(search::<i32>(root, &[1, 2, 4]), Some(&124));

        assert_eq!(search::<i32>(root, &[]), None);
        assert_eq!(search::<i32>(root, &[1, 2]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 10]), None);
        assert_eq!(search::<i32>(root, &[0, 2, 1]), None);
    }
}

#[test]
fn lookup_on_empty_nodes() {
    let mut n4 = InnerNode4::empty();
    let mut n16 = InnerNode16::empty();
    let mut n48 = InnerNode48::empty();
    let mut n256 = InnerNode256::empty();

    let roots = vec![
        NodePtr::from(&mut n4).to_opaque(),
        NodePtr::from(&mut n16).to_opaque(),
        NodePtr::from(&mut n48).to_opaque(),
        NodePtr::from(&mut n256).to_opaque(),
    ];

    for root in roots {
        // SAFETY: All the `search` calls are safe because there are no leaves in this
        // tree.
        unsafe {
            assert_eq!(search::<()>(root, &[1, 2, 1]), None);
            assert_eq!(search::<()>(root, &[1, 2, 2]), None);
            assert_eq!(search::<()>(root, &[1, 2, 3]), None);
            assert_eq!(search::<()>(root, &[1, 2, 4]), None);
            assert_eq!(search::<()>(root, &[]), None);
            assert_eq!(search::<()>(root, &[1, 2]), None);
            assert_eq!(search::<()>(root, &[1, 2, 10]), None);
            assert_eq!(search::<()>(root, &[0, 2, 1]), None);
        }
    }
}

#[test]
fn lookup_on_node16() {
    let mut l1: LeafNode<i32> = LeafNode::new(vec![1, 2, 1], 121);
    let mut l2: LeafNode<i32> = LeafNode::new(vec![1, 2, 2], 122);
    let mut l3: LeafNode<i32> = LeafNode::new(vec![1, 2, 3], 123);
    let mut l4: LeafNode<i32> = LeafNode::new(vec![1, 2, 4], 124);

    let l1_ptr = NodePtr::from(&mut l1).to_opaque();
    let l2_ptr = NodePtr::from(&mut l2).to_opaque();
    let l3_ptr = NodePtr::from(&mut l3).to_opaque();
    let l4_ptr = NodePtr::from(&mut l4).to_opaque();

    let mut inner_node = InnerNode16::empty();

    // Update inner node prefix and child slots
    inner_node.header.write_prefix(&[1, 2]);
    inner_node.write_child(1, l1_ptr);
    inner_node.write_child(2, l2_ptr);
    inner_node.write_child(3, l3_ptr);
    inner_node.write_child(4, l4_ptr);

    let root = NodePtr::from(&mut inner_node).to_opaque();

    // SAFETY: All the `search` calls are safe because they are specifing the `i32`
    // output type parameter, the same one that was used to construct the leaf
    // nodes.
    unsafe {
        assert_eq!(search::<i32>(root, &[1, 2, 1]), Some(&121));
        assert_eq!(search::<i32>(root, &[1, 2, 2]), Some(&122));
        assert_eq!(search::<i32>(root, &[1, 2, 3]), Some(&123));
        assert_eq!(search::<i32>(root, &[1, 2, 4]), Some(&124));

        assert_eq!(search::<i32>(root, &[]), None);
        assert_eq!(search::<i32>(root, &[1, 2]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 10]), None);
        assert_eq!(search::<i32>(root, &[0, 2, 1]), None);
    }
}

#[test]
fn lookup_on_node48() {
    let mut l1: LeafNode<i32> = LeafNode::new(vec![1, 2, 1], 121);
    let mut l2: LeafNode<i32> = LeafNode::new(vec![1, 2, 2], 122);
    let mut l3: LeafNode<i32> = LeafNode::new(vec![1, 2, 3], 123);
    let mut l4: LeafNode<i32> = LeafNode::new(vec![1, 2, 4], 124);

    let l1_ptr = NodePtr::from(&mut l1).to_opaque();
    let l2_ptr = NodePtr::from(&mut l2).to_opaque();
    let l3_ptr = NodePtr::from(&mut l3).to_opaque();
    let l4_ptr = NodePtr::from(&mut l4).to_opaque();

    let mut inner_node = InnerNode48::empty();

    // Update inner node prefix and child slots
    inner_node.header.write_prefix(&[1, 2]);
    inner_node.write_child(1, l1_ptr);
    inner_node.write_child(2, l2_ptr);
    inner_node.write_child(3, l3_ptr);
    inner_node.write_child(4, l4_ptr);

    let root = NodePtr::from(&mut inner_node).to_opaque();

    // SAFETY: All the `search` calls are safe because they are specifing the `i32`
    // output type parameter, the same one that was used to construct the leaf
    // nodes.
    unsafe {
        assert_eq!(search::<i32>(root, &[1, 2, 1]), Some(&121));
        assert_eq!(search::<i32>(root, &[1, 2, 2]), Some(&122));
        assert_eq!(search::<i32>(root, &[1, 2, 3]), Some(&123));
        assert_eq!(search::<i32>(root, &[1, 2, 4]), Some(&124));

        assert_eq!(search::<i32>(root, &[]), None);
        assert_eq!(search::<i32>(root, &[1, 2]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 10]), None);
        assert_eq!(search::<i32>(root, &[0, 2, 1]), None);
    }
}

#[test]
fn lookup_on_node256() {
    let mut l1: LeafNode<i32> = LeafNode::new(vec![1, 2, 1], 121);
    let mut l2: LeafNode<i32> = LeafNode::new(vec![1, 2, 2], 122);
    let mut l3: LeafNode<i32> = LeafNode::new(vec![1, 2, 3], 123);
    let mut l4: LeafNode<i32> = LeafNode::new(vec![1, 2, 4], 124);

    let l1_ptr = NodePtr::from(&mut l1).to_opaque();
    let l2_ptr = NodePtr::from(&mut l2).to_opaque();
    let l3_ptr = NodePtr::from(&mut l3).to_opaque();
    let l4_ptr = NodePtr::from(&mut l4).to_opaque();

    let mut inner_node = InnerNode256::empty();

    // Update inner node prefix and child slots
    inner_node.header.write_prefix(&[1, 2]);
    inner_node.write_child(1, l1_ptr);
    inner_node.write_child(2, l2_ptr);
    inner_node.write_child(3, l3_ptr);
    inner_node.write_child(4, l4_ptr);

    let root = NodePtr::from(&mut inner_node).to_opaque();

    // SAFETY: All the `search` calls are safe because they are specifing the `i32`
    // output type parameter, the same one that was used to construct the leaf
    // nodes.
    unsafe {
        assert_eq!(search::<i32>(root, &[1, 2, 1]), Some(&121));
        assert_eq!(search::<i32>(root, &[1, 2, 2]), Some(&122));
        assert_eq!(search::<i32>(root, &[1, 2, 3]), Some(&123));
        assert_eq!(search::<i32>(root, &[1, 2, 4]), Some(&124));

        assert_eq!(search::<i32>(root, &[]), None);
        assert_eq!(search::<i32>(root, &[1, 2]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 10]), None);
        assert_eq!(search::<i32>(root, &[0, 2, 1]), None);
    }
}

#[test]
fn lookup_on_n16_n4_layer_tree() {
    // ┌──────────┬────────┐     ┌──────────┬────────┐     ┌───────┬──────────────┐
    // │  Prefix  │  1,2   │  ┌─▶│  Prefix  │  5,6   │  ┌─▶│ Type  │     leaf     │
    // ├──────────┼────────┤  │  ├──────────┼────────┤  │  ├───────┼──────────────┤
    // │   Type   │  n16   │  │  │   Type   │   n4   │  │  │  Key  │ 1,2,3,5,6,1  │
    // ├──────────┼────────┤  │  ├──────────┼────────┤  │  ├───────┼──────────────┤
    // │          │   3    │──┘  │          │   1    │──┘  │ Value │    123561    │
    // │ Children ├────────┤     │ Children ├────────┤     └───────┴──────────────┘
    // │          │   4    │──┐  │          │   2    │──┐  ┌───────┬──────────────┐
    // └──────────┴────────┘  │  └──────────┴────────┘  └─▶│ Type  │     leaf     │
    //                        │  ┌──────────┬────────┐     ├───────┼──────────────┤
    //                        └─▶│  Prefix  │  7,8   │     │  Key  │ 1,2,3,5,6,2  │
    //                           ├──────────┼────────┤     ├───────┼──────────────┤
    //                           │   Type   │   n4   │     │ Value │    124784    │
    //                           ├──────────┼────────┤     └───────┴──────────────┘
    //                           │          │   3    │──┐  ┌───────┬──────────────┐
    //                           │ Children ├────────┤  └─▶│ Type  │     leaf     │
    //                           │          │   4    │──┐  ├───────┼──────────────┤
    //                           └──────────┴────────┘  │  │  Key  │ 1,2,4,7,8,3  │
    //                                                  │  ├───────┼──────────────┤
    //                                                  │  │ Value │    124783    │
    //                                                  │  └───────┴──────────────┘
    //                                                  │  ┌───────┬──────────────┐
    //                                                  └─▶│ Type  │     leaf     │
    //                                                     ├───────┼──────────────┤
    //                                                     │  Key  │ 1,2,4,7,8,4  │
    //                                                     ├───────┼──────────────┤
    //                                                     │ Value │    124784    │
    //                                                     └───────┴──────────────┘

    let mut l1: LeafNode<i32> = LeafNode::new(vec![1, 2, 3, 5, 6, 1], 123561);
    let mut l2: LeafNode<i32> = LeafNode::new(vec![1, 2, 3, 5, 6, 2], 123562);
    let mut l3: LeafNode<i32> = LeafNode::new(vec![1, 2, 4, 7, 8, 3], 124783);
    let mut l4: LeafNode<i32> = LeafNode::new(vec![1, 2, 4, 7, 8, 4], 124784);

    let l1_ptr = NodePtr::from(&mut l1).to_opaque();
    let l2_ptr = NodePtr::from(&mut l2).to_opaque();
    let l3_ptr = NodePtr::from(&mut l3).to_opaque();
    let l4_ptr = NodePtr::from(&mut l4).to_opaque();

    let mut n4_left = InnerNode4::empty();
    let mut n4_right = InnerNode4::empty();
    let mut n16 = InnerNode16::empty();

    // Update inner node prefix and child slots
    n4_left.header.write_prefix(&[5, 6]);
    n4_left.write_child(1, l1_ptr);
    n4_left.write_child(2, l2_ptr);

    n4_right.header.write_prefix(&[7, 8]);
    n4_right.write_child(3, l3_ptr);
    n4_right.write_child(4, l4_ptr);

    let n4_left_ptr = NodePtr::from(&mut n4_left).to_opaque();
    let n4_right_ptr = NodePtr::from(&mut n4_right).to_opaque();

    n16.header.write_prefix(&[1, 2]);
    n16.write_child(3, n4_left_ptr);
    n16.write_child(4, n4_right_ptr);

    let root = NodePtr::from(&mut n16).to_opaque();

    // SAFETY: All the `search` calls are safe because they are specifing the `i32`
    // output type parameter, the same one that was used to construct the leaf
    // nodes.
    unsafe {
        assert_eq!(search::<i32>(root, &[1, 2, 3, 5, 6, 1]), Some(&123561));
        assert_eq!(search::<i32>(root, &[1, 2, 3, 5, 6, 2]), Some(&123562));
        assert_eq!(search::<i32>(root, &[1, 2, 4, 7, 8, 3]), Some(&124783));
        assert_eq!(search::<i32>(root, &[1, 2, 4, 7, 8, 4]), Some(&124784));

        assert_eq!(search::<i32>(root, &[1, 2, 3]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 4]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 3, 5, 6]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 4, 7, 8]), None);

        assert_eq!(search::<i32>(root, &[1, 2, 3, 50, 6, 1]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 4, 70, 8, 3]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 30, 5, 6, 1]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 40, 7, 8, 3]), None);
        assert_eq!(search::<i32>(root, &[10, 2, 3, 5, 6, 1]), None);
        assert_eq!(search::<i32>(root, &[1, 20, 4, 7, 8, 3]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 3, 5, 60, 1]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 4, 7, 80, 3]), None);
    }
}

#[test]
fn lookup_on_n48_n4_layer_tree() {
    let mut l1: LeafNode<i32> = LeafNode::new(vec![1, 2, 3, 5, 6, 1], 123561);
    let mut l2: LeafNode<i32> = LeafNode::new(vec![1, 2, 3, 5, 6, 2], 123562);
    let mut l3: LeafNode<i32> = LeafNode::new(vec![1, 2, 4, 7, 8, 3], 124783);
    let mut l4: LeafNode<i32> = LeafNode::new(vec![1, 2, 4, 7, 8, 4], 124784);

    let l1_ptr = NodePtr::from(&mut l1).to_opaque();
    let l2_ptr = NodePtr::from(&mut l2).to_opaque();
    let l3_ptr = NodePtr::from(&mut l3).to_opaque();
    let l4_ptr = NodePtr::from(&mut l4).to_opaque();

    let mut n4_left = InnerNode4::empty();
    let mut n4_right = InnerNode4::empty();
    let mut n48 = InnerNode48::empty();

    // Update inner node prefix and child slots
    n4_left.header.write_prefix(&[5, 6]);
    n4_left.write_child(1, l1_ptr);
    n4_left.write_child(2, l2_ptr);

    n4_right.header.write_prefix(&[7, 8]);
    n4_right.write_child(3, l3_ptr);
    n4_right.write_child(4, l4_ptr);

    let n4_left_ptr = NodePtr::from(&mut n4_left).to_opaque();
    let n4_right_ptr = NodePtr::from(&mut n4_right).to_opaque();

    n48.header.write_prefix(&[1, 2]);
    n48.write_child(3, n4_left_ptr);
    n48.write_child(4, n4_right_ptr);

    let root = NodePtr::from(&mut n48).to_opaque();

    // SAFETY: All the `search` calls are safe because they are specifing the `i32`
    // output type parameter, the same one that was used to construct the leaf
    // nodes.
    unsafe {
        assert_eq!(search::<i32>(root, &[1, 2, 3, 5, 6, 1]), Some(&123561));
        assert_eq!(search::<i32>(root, &[1, 2, 3, 5, 6, 2]), Some(&123562));
        assert_eq!(search::<i32>(root, &[1, 2, 4, 7, 8, 3]), Some(&124783));
        assert_eq!(search::<i32>(root, &[1, 2, 4, 7, 8, 4]), Some(&124784));

        assert_eq!(search::<i32>(root, &[1, 2, 3]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 4]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 3, 5, 6]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 4, 7, 8]), None);

        assert_eq!(search::<i32>(root, &[1, 2, 3, 50, 6, 1]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 4, 70, 8, 3]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 30, 5, 6, 1]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 40, 7, 8, 3]), None);
        assert_eq!(search::<i32>(root, &[10, 2, 3, 5, 6, 1]), None);
        assert_eq!(search::<i32>(root, &[1, 20, 4, 7, 8, 3]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 3, 5, 60, 1]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 4, 7, 80, 3]), None);
    }
}

#[test]
fn lookup_on_n256_n4_layer_tree() {
    let mut l1: LeafNode<i32> = LeafNode::new(vec![1, 2, 3, 5, 6, 1], 123561);
    let mut l2: LeafNode<i32> = LeafNode::new(vec![1, 2, 3, 5, 6, 2], 123562);
    let mut l3: LeafNode<i32> = LeafNode::new(vec![1, 2, 4, 7, 8, 3], 124783);
    let mut l4: LeafNode<i32> = LeafNode::new(vec![1, 2, 4, 7, 8, 4], 124784);

    let l1_ptr = NodePtr::from(&mut l1).to_opaque();
    let l2_ptr = NodePtr::from(&mut l2).to_opaque();
    let l3_ptr = NodePtr::from(&mut l3).to_opaque();
    let l4_ptr = NodePtr::from(&mut l4).to_opaque();

    let mut n4_left = InnerNode4::empty();
    let mut n4_right = InnerNode4::empty();
    let mut n256 = InnerNode256::empty();

    // Update inner node prefix and child slots
    n4_left.header.write_prefix(&[5, 6]);
    n4_left.write_child(1, l1_ptr);
    n4_left.write_child(2, l2_ptr);

    n4_right.header.write_prefix(&[7, 8]);
    n4_right.write_child(3, l3_ptr);
    n4_right.write_child(4, l4_ptr);

    let n4_left_ptr = NodePtr::from(&mut n4_left).to_opaque();
    let n4_right_ptr = NodePtr::from(&mut n4_right).to_opaque();

    n256.header.write_prefix(&[1, 2]);
    n256.write_child(3, n4_left_ptr);
    n256.write_child(4, n4_right_ptr);

    let root = NodePtr::from(&mut n256).to_opaque();

    // SAFETY: All the `search` calls are safe because they are specifing the `i32`
    // output type parameter, the same one that was used to construct the leaf
    // nodes.
    unsafe {
        assert_eq!(search::<i32>(root, &[1, 2, 3, 5, 6, 1]), Some(&123561));
        assert_eq!(search::<i32>(root, &[1, 2, 3, 5, 6, 2]), Some(&123562));
        assert_eq!(search::<i32>(root, &[1, 2, 4, 7, 8, 3]), Some(&124783));
        assert_eq!(search::<i32>(root, &[1, 2, 4, 7, 8, 4]), Some(&124784));

        assert_eq!(search::<i32>(root, &[1, 2, 3]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 4]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 3, 5, 6]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 4, 7, 8]), None);

        assert_eq!(search::<i32>(root, &[1, 2, 3, 50, 6, 1]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 4, 70, 8, 3]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 30, 5, 6, 1]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 40, 7, 8, 3]), None);
        assert_eq!(search::<i32>(root, &[10, 2, 3, 5, 6, 1]), None);
        assert_eq!(search::<i32>(root, &[1, 20, 4, 7, 8, 3]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 3, 5, 60, 1]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 4, 7, 80, 3]), None);
    }
}

#[test]
fn lookup_on_n4_n4_layer_tree() {
    let mut l1: LeafNode<i32> = LeafNode::new(vec![1, 2, 3, 5, 6, 1], 123561);
    let mut l2: LeafNode<i32> = LeafNode::new(vec![1, 2, 3, 5, 6, 2], 123562);
    let mut l3: LeafNode<i32> = LeafNode::new(vec![1, 2, 4, 7, 8, 3], 124783);
    let mut l4: LeafNode<i32> = LeafNode::new(vec![1, 2, 4, 7, 8, 4], 124784);

    let l1_ptr = NodePtr::from(&mut l1).to_opaque();
    let l2_ptr = NodePtr::from(&mut l2).to_opaque();
    let l3_ptr = NodePtr::from(&mut l3).to_opaque();
    let l4_ptr = NodePtr::from(&mut l4).to_opaque();

    let mut n4_left = InnerNode4::empty();
    let mut n4_right = InnerNode4::empty();
    let mut n4 = InnerNode4::empty();

    // Update inner node prefix and child slots
    n4_left.header.write_prefix(&[5, 6]);
    n4_left.write_child(1, l1_ptr);
    n4_left.write_child(2, l2_ptr);

    n4_right.header.write_prefix(&[7, 8]);
    n4_right.write_child(3, l3_ptr);
    n4_right.write_child(4, l4_ptr);

    let n4_left_ptr = NodePtr::from(&mut n4_left).to_opaque();
    let n4_right_ptr = NodePtr::from(&mut n4_right).to_opaque();

    n4.header.write_prefix(&[1, 2]);
    n4.write_child(3, n4_left_ptr);
    n4.write_child(4, n4_right_ptr);

    let root = NodePtr::from(&mut n4).to_opaque();

    // SAFETY: All the `search` calls are safe because they are specifing the `i32`
    // output type parameter, the same one that was used to construct the leaf
    // nodes.
    unsafe {
        assert_eq!(search::<i32>(root, &[1, 2, 3, 5, 6, 1]), Some(&123561));
        assert_eq!(search::<i32>(root, &[1, 2, 3, 5, 6, 2]), Some(&123562));
        assert_eq!(search::<i32>(root, &[1, 2, 4, 7, 8, 3]), Some(&124783));
        assert_eq!(search::<i32>(root, &[1, 2, 4, 7, 8, 4]), Some(&124784));

        assert_eq!(search::<i32>(root, &[1, 2, 3]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 4]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 3, 5, 6]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 4, 7, 8]), None);

        assert_eq!(search::<i32>(root, &[1, 2, 3, 50, 6, 1]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 4, 70, 8, 3]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 30, 5, 6, 1]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 40, 7, 8, 3]), None);
        assert_eq!(search::<i32>(root, &[10, 2, 3, 5, 6, 1]), None);
        assert_eq!(search::<i32>(root, &[1, 20, 4, 7, 8, 3]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 3, 5, 60, 1]), None);
        assert_eq!(search::<i32>(root, &[1, 2, 4, 7, 80, 3]), None);
    }
}
