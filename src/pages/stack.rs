//! Keep GTK's child traversal and GDK's window order in agreement.
use glib::subclass::prelude::*;
use gtk::{prelude::*, subclass::prelude::*};
use std::cell::{Cell, RefCell};

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct Stack {
        pub collecting: Cell<bool>,
        pub chrome: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
        pub feedback: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
        pub held: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for Stack {
        const NAME: &'static str = "CanopyPageStack";
        type Type = super::Stack;
        type ParentType = gtk::Fixed;
    }
    impl ObjectImpl for Stack {}
    impl WidgetImpl for Stack {
        fn draw(&self, cr: &gtk::cairo::Context) -> glib::Propagation {
            // GtkFixed draws its private insertion list. Use GtkContainer's standard
            // propagation so the same ordered traversal controls drawing and stacking.
            use glib::translate::*;
            unsafe {
                let class =
                    glib::gobject_ffi::g_type_class_ref(gtk::Container::static_type().into_glib())
                        as *mut gtk::ffi::GtkWidgetClass;
                let result = ((*class).draw.unwrap())(
                    self.obj().upcast_ref::<gtk::Widget>().to_glib_none().0,
                    cr.to_glib_none().0,
                );
                glib::gobject_ffi::g_type_class_unref(class.cast());
                from_glib(result)
            }
        }
    }
    impl ContainerImpl for Stack {
        fn forall(&self, internal: bool, callback: &gtk::subclass::container::Callback) {
            if self.collecting.replace(true) {
                self.parent_forall(internal, callback);
                return;
            }
            let mut children = self.obj().children();
            self.collecting.set(false);
            let chrome = self.chrome.borrow().as_ref().and_then(|w| w.upgrade());
            let feedback = self.feedback.borrow().as_ref().and_then(|w| w.upgrade());
            let held = self.held.borrow().as_ref().and_then(|w| w.upgrade());
            children.sort_by_key(|w| {
                if Some(w) == feedback.as_ref() {
                    3
                } else if Some(w) == chrome.as_ref() {
                    2
                } else if Some(w) == held.as_ref() {
                    1
                } else {
                    0
                }
            });
            for child in children {
                callback.call(&child);
            }
        }
    }
    impl FixedImpl for Stack {}
}
glib::wrapper! {
    pub struct Stack(ObjectSubclass<imp::Stack>) @extends gtk::Fixed, gtk::Container, gtk::Widget;
}
impl Stack {
    pub(crate) fn fixed() -> gtk::Fixed {
        glib::Object::new::<Self>().upcast()
    }
    pub(crate) fn feedback(container: &gtk::Fixed, widget: &gtk::Widget) {
        if let Some(stack) = container.downcast_ref::<Self>() {
            *stack.imp().feedback.borrow_mut() = Some(widget.downgrade());
        }
    }

    pub(crate) fn order(
        container: &gtk::Fixed,
        chrome: Option<&gtk::Widget>,
        held: Option<&gtk::Widget>,
    ) {
        let Some(stack) = container.downcast_ref::<Self>() else {
            return;
        };
        *stack.imp().chrome.borrow_mut() = chrome.map(|w| w.downgrade());
        *stack.imp().held.borrow_mut() = held.map(|w| w.downgrade());
        for child in stack.children() {
            if let Some(window) = child.window() {
                window.raise();
            }
        }
        stack.queue_draw();
    }
}
