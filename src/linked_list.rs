use core::marker::PhantomData;
use core::ptr::NonNull;

/// An instrusive double-linked list.
#[derive(Debug)]
pub(crate) struct LinkedList<T>
where
    T: Link,
{
    head: Option<NonNull<T>>,
    tail: Option<NonNull<T>>,

    _marker: PhantomData<*const T>,

    #[cfg(debug_assertions)]
    len: usize,
}

/// The linked elements in a [`LinkedList`].
///
/// # Safety
///
/// Implementations of `Link` must be pinned in memory. When a node is inserted it must not be
/// moved until is was removed again.
pub(crate) unsafe trait Link {
    fn next(&self) -> Option<NonNull<Self>>;
    fn prev(&self) -> Option<NonNull<Self>>;

    fn next_mut(&mut self) -> &mut Option<NonNull<Self>>;
    fn prev_mut(&mut self) -> &mut Option<NonNull<Self>>;
}

impl<T> LinkedList<T>
where
    T: Link,
{
    /// Creates a new, empty `LinkedList`.
    #[inline]
    pub const fn new() -> Self {
        Self {
            head: None,
            tail: None,
            _marker: PhantomData,

            #[cfg(debug_assertions)]
            len: 0,
        }
    }

    /// Pushes a new element to the front of the list.
    ///
    /// # Safety
    ///
    /// The pushed element must live as long as the linked list, or be removed before it is
    /// dropped. Dropping the element before removing it from the list is undefined behavoir.
    #[allow(unused)]
    pub unsafe fn push_front(&mut self, ptr: NonNull<T>) {
        let node = &mut *ptr.as_ptr();

        *node.next_mut() = self.head;
        *node.prev_mut() = None;

        match self.head {
            None => self.tail = Some(ptr),
            Some(head) => *(*head.as_ptr()).prev_mut() = Some(ptr),
        }

        self.head = Some(ptr);

        #[cfg(debug_assertions)]
        {
            self.len += 1;
            if self.len >= 2 {
                assert_ne!(self.head, self.tail);
            }
        }
    }

    /// Pushes a new element to the back of the list.
    ///
    /// # Safety
    ///
    /// The pushed element must live as long as the linked list, or be remove before it is
    /// dropped. Dropping the element before removing it from the list undefined behavoir.
    pub unsafe fn push_back(&mut self, ptr: NonNull<T>) {
        let node = &mut *ptr.as_ptr();

        *node.next_mut() = None;
        *node.prev_mut() = self.tail;

        match self.tail {
            None => self.head = Some(ptr),
            Some(tail) => *(*tail.as_ptr()).next_mut() = Some(ptr),
        }

        self.tail = Some(ptr);

        #[cfg(debug_assertions)]
        {
            self.len += 1;
            if self.len > 1 {
                assert_ne!(self.head, self.tail);
            }
        }
    }

    /// Removes the element with the given pointer from the `LinkedList`.
    ///
    /// # Safety
    ///
    /// `ptr` must be a valid pointer that exists inside the list. Using any type of invalid
    /// pointer (inproper alignment, dangling, etc..) is undefined behavoir.
    pub unsafe fn remove(&mut self, ptr: NonNull<T>) {
        #[cfg(debug_assertions)]
        {
            self.len -= 1;
        }

        let node = &mut *ptr.as_ptr();

        match node.next() {
            Some(next) => *(*next.as_ptr()).prev_mut() = node.prev(),
            None => self.tail = node.prev(),
        }

        match node.prev() {
            Some(prev) => *(*prev.as_ptr()).next_mut() = node.next(),
            None => self.head = node.next(),
        }
    }

    /// Returns `true` if the list is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        if self.head.is_some() {
            return false;
        }

        #[cfg(debug_assertions)]
        {
            assert!(self.tail.is_none());
        }

        true
    }

    #[inline]
    pub fn iter_mut(&mut self) -> IterMut<'_, T> {
        IterMut {
            head: self.head,
            _marker: PhantomData,
        }
    }

    #[inline]
    pub fn front(&self) -> Option<&T> {
        unsafe { self.head.map(|ptr| &*ptr.as_ptr()) }
    }
}

impl<T> Clone for LinkedList<T>
where
    T: Link,
{
    #[inline]
    fn clone(&self) -> Self {
        Self {
            head: self.head,
            tail: self.tail,
            _marker: PhantomData,

            #[cfg(debug_assertions)]
            len: self.len,
        }
    }
}

impl<T> Default for LinkedList<T>
where
    T: Link,
{
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Drop for LinkedList<T>
where
    T: Link,
{
    #[inline]
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        {
            assert_eq!(self.len, 0);
            assert!(self.is_empty());
        }
    }
}

unsafe impl<T> Send for LinkedList<T> where T: Link + Send {}
unsafe impl<T> Sync for LinkedList<T> where T: Link + Sync {}

#[derive(Debug)]
pub(crate) struct IterMut<'a, T>
where
    T: Link,
{
    head: Option<NonNull<T>>,

    _marker: PhantomData<&'a mut T>,
}

impl<'a, T> Iterator for IterMut<'a, T>
where
    T: Link,
{
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        let head = self.head?;

        unsafe {
            let node = &mut *head.as_ptr();
            self.head = node.next();
            Some(node)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::mem;
    use std::{cell::UnsafeCell, ops::Deref, ptr::NonNull};

    use super::{Link, LinkedList};

    #[derive(Debug, Default)]
    struct Node(UnsafeCell<NodeInner>);

    unsafe impl Link for Node {
        fn next(&self) -> Option<NonNull<Self>> {
            unsafe { (&*self.0.get()).next }
        }

        fn prev(&self) -> Option<NonNull<Self>> {
            unsafe { (&*self.0.get()).prev }
        }

        fn next_mut(&mut self) -> &mut Option<NonNull<Self>> {
            unsafe { &mut (&mut *self.0.get()).next }
        }

        fn prev_mut(&mut self) -> &mut Option<NonNull<Self>> {
            unsafe { &mut (&mut *self.0.get()).prev }
        }
    }

    impl Deref for Node {
        type Target = NodeInner;

        fn deref(&self) -> &Self::Target {
            unsafe { &*self.0.get() }
        }
    }

    #[derive(Clone, Debug, Default)]
    struct NodeInner {
        next: Option<NonNull<Node>>,
        prev: Option<NonNull<Node>>,
    }

    #[test]
    fn test_linked_list_push_front() {
        let mut list = LinkedList::new();

        let node = Node::default();
        unsafe {
            list.push_front((&node).into());
        }

        assert_eq!(list.head, Some((&node).into()));
        assert_eq!(list.tail, Some((&node).into()));

        assert_eq!(node.next, None);
        assert_eq!(node.prev, None);

        let node2 = Node::default();
        unsafe {
            list.push_front((&node2).into());
        }

        assert_eq!(list.head, Some((&node2).into()));
        assert_eq!(list.tail, Some((&node).into()));

        assert_eq!(node.next, None);
        assert_eq!(node.prev, Some((&node2).into()));

        assert_eq!(node2.next, Some((&node).into()));
        assert_eq!(node2.prev, None);

        let node3 = Node::default();
        unsafe {
            list.push_front((&node3).into());
        }

        assert_eq!(list.head, Some((&node3).into()));
        assert_eq!(list.tail, Some((&node).into()));

        assert_eq!(node.next, None);
        assert_eq!(node.prev, Some((&node2).into()));

        assert_eq!(node2.next, Some((&node).into()));
        assert_eq!(node2.prev, Some((&node3).into()));

        assert_eq!(node3.next, Some((&node2).into()));
        assert_eq!(node3.prev, None);

        // Destroy the list without asserting that it is empty.
        mem::forget(list);
    }

    #[test]
    fn test_linked_list_push_back() {
        let mut list = LinkedList::new();

        let node = Node::default();
        unsafe {
            list.push_back((&node).into());
        }

        assert_eq!(list.head, Some((&node).into()));
        assert_eq!(list.tail, Some((&node).into()));

        assert_eq!(node.next, None);
        assert_eq!(node.prev, None);

        let node2 = Node::default();
        unsafe {
            list.push_back((&node2).into());
        }

        assert_eq!(list.head, Some((&node).into()));
        assert_eq!(list.tail, Some((&node2).into()));

        assert_eq!(node.next, Some((&node2).into()));
        assert_eq!(node.prev, None);

        assert_eq!(node2.next, None);
        assert_eq!(node2.prev, Some((&node).into()));

        let node3 = Node::default();
        unsafe {
            list.push_back((&node3).into());
        }

        assert_eq!(list.head, Some((&node).into()));
        assert_eq!(list.tail, Some((&node3).into()));

        assert_eq!(node.next, Some((&node2).into()));
        assert_eq!(node.prev, None);

        assert_eq!(node2.next, Some((&node3).into()));
        assert_eq!(node2.prev, Some((&node).into()));

        assert_eq!(node3.next, None);
        assert_eq!(node3.prev, Some((&node2).into()));

        // Destroy the list without asserting that it is empty.
        mem::forget(list);
    }

    #[test]
    fn test_linked_list_remove() {
        let node = Node::default();
        let node2 = Node::default();
        let node3 = Node::default();

        // Remove the first element (list.head).
        let mut list = LinkedList::new();
        unsafe {
            list.push_back((&node).into());
            list.push_back((&node2).into());
            list.push_back((&node3).into());
            list.remove((&node).into());
        }

        assert_eq!(list.head, Some((&node2).into()));
        assert_eq!(list.tail, Some((&node3).into()));

        assert_eq!(node2.next, Some((&node3).into()));
        assert_eq!(node2.prev, None);

        assert_eq!(node3.next, None);
        assert_eq!(node3.prev, Some((&node2).into()));

        // Destroy the list without asserting that it is empty.
        mem::forget(list);

        // Remove the last element (list.tail).
        let mut list = LinkedList::new();
        unsafe {
            list.push_back((&node).into());
            list.push_back((&node2).into());
            list.push_back((&node3).into());
            list.remove((&node3).into());
        }

        assert_eq!(list.head, Some((&node).into()));
        assert_eq!(list.tail, Some((&node2).into()));

        assert_eq!(node.next, Some((&node2).into()));
        assert_eq!(node.prev, None);

        assert_eq!(node2.next, None);
        assert_eq!(node2.prev, Some((&node).into()));

        // Destroy the list without asserting that it is empty.
        mem::forget(list);

        // Remove from middle.
        let mut list = LinkedList::new();
        unsafe {
            list.push_back((&node).into());
            list.push_back((&node2).into());
            list.push_back((&node3).into());
            list.remove((&node2).into());
        }

        assert_eq!(list.head, Some((&node).into()));
        assert_eq!(list.tail, Some((&node3).into()));

        assert_eq!(node.next, Some((&node3).into()));
        assert_eq!(node.prev, None);

        assert_eq!(node3.next, None);
        assert_eq!(node3.prev, Some((&node).into()));

        // Destroy the list without asserting that it is empty.
        mem::forget(list);
    }
}
