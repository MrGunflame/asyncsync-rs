use core::marker::PhantomData;
use core::ptr::NonNull;
use std::ops::{Deref, DerefMut};

#[derive(Debug)]
pub struct LinkedList<T> {
    head: Option<NonNull<Node<T>>>,
    tail: Option<NonNull<Node<T>>>,

    _marker: PhantomData<*const T>,
}

impl<T> LinkedList<T> {
    pub fn new() -> Self {
        Self {
            head: None,
            tail: None,
            _marker: PhantomData,
        }
    }

    pub fn push_front(&mut self, val: T) -> NonNull<Node<T>> {
        self.push_front_node(Box::new(Node::new(val)))
    }

    pub fn push_back(&mut self, val: T) -> NonNull<Node<T>> {
        self.push_back_node(Box::new(Node::new(val)))
    }

    pub fn iter_mut(&mut self) -> IterMut<'_, T> {
        IterMut {
            head: self.head,
            tail: self.tail,
            _marker: PhantomData,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.head.is_none()
    }

    pub fn front(&self) -> Option<&T> {
        unsafe {
            match self.head {
                Some(head) => Some(&*head.as_ptr()),
                None => None,
            }
        }
    }

    pub fn back(&self) -> Option<&T> {
        unsafe {
            match self.tail {
                Some(tail) => Some(&*tail.as_ptr()),
                None => None,
            }
        }
    }

    pub fn remove(&mut self, node: &mut Node<T>) {
        println!("rm");

        unsafe {
            if let Some(prev) = node.prev {
                (*prev.as_ptr()).next = node.next;
            } else {
                self.head = node.next;
            }

            if let Some(next) = node.next {
                (*next.as_ptr()).prev = node.prev;
            } else {
                self.tail = node.prev;
            }
        }
    }

    fn push_front_node(&mut self, mut node: Box<Node<T>>) -> NonNull<Node<T>> {
        unsafe {
            node.next = self.head;
            node.prev = None;
            let ptr = Box::leak(node).into();
            let node = Some(ptr);

            match self.head {
                None => self.tail = node,
                Some(head) => (*head.as_ptr()).prev = node,
            }

            self.head = node;
            ptr
        }
    }

    fn push_back_node(&mut self, mut node: Box<Node<T>>) -> NonNull<Node<T>> {
        unsafe {
            node.next = None;
            node.prev = self.tail;
            let ptr = Box::leak(node).into();
            let node = Some(ptr);

            match self.tail {
                None => self.head = node,
                Some(tail) => (*tail.as_ptr()).next = node,
            }

            self.tail = node;
            ptr
        }
    }
}

impl<T> Default for LinkedList<T> {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl<T> Send for LinkedList<T> where T: Send {}
unsafe impl<T> Sync for LinkedList<T> where T: Sync {}

pub trait Link<T> {
    fn next() -> Option<NonNull<T>>;
    fn prev() -> Option<NonNull<T>>;
}

pub struct Node<T> {
    prev: Option<NonNull<Node<T>>>,
    next: Option<NonNull<Node<T>>>,
    elem: T,
}

impl<T> Node<T> {
    fn new(elem: T) -> Self {
        Self {
            prev: None,
            next: None,
            elem,
        }
    }
}

impl<T> Deref for Node<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.elem
    }
}

impl<T> DerefMut for Node<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.elem
    }
}

pub struct IterMut<'a, T> {
    head: Option<NonNull<Node<T>>>,
    tail: Option<NonNull<Node<T>>>,

    _marker: PhantomData<&'a mut Node<T>>,
}

impl<'a, T> Iterator for IterMut<'a, T> {
    type Item = &'a mut T;

    fn next(&mut self) -> Option<Self::Item> {
        let head = self.head?;

        unsafe {
            let node = &mut *head.as_ptr();
            self.head = node.next;
            Some(&mut node.elem)
        }
    }
}
