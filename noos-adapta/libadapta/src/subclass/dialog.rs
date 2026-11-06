use glib::subclass::prelude::*;
use glib::translate::*;

use crate::{Dialog, prelude::*};
use gtk::subclass::prelude::WidgetImpl;

pub trait AdapDialogImpl: WidgetImpl + ObjectSubclass<Type: IsA<Dialog>> {
    fn close_attempt(&self) {
        AdapDialogImplExt::parent_close_attempt(self)
    }

    fn closed(&self) {
        AdapDialogImplExt::parent_closed(self)
    }
}

pub trait AdapDialogImplExt: AdapDialogImpl {
    fn parent_close_attempt(&self) {
        unsafe {
            let data = Self::type_data();
            let parent_class = data.as_ref().parent_class() as *mut ffi::AdapDialogClass;
            if let Some(f) = (*parent_class).close_attempt {
                f(self.obj().unsafe_cast_ref::<Dialog>().to_glib_none().0)
            }
        }
    }

    fn parent_closed(&self) {
        unsafe {
            let data = Self::type_data();
            let parent_class = data.as_ref().parent_class() as *mut ffi::AdapDialogClass;
            if let Some(f) = (*parent_class).closed {
                f(self.obj().unsafe_cast_ref::<Dialog>().to_glib_none().0)
            }
        }
    }
}

impl<T: AdapDialogImpl> AdapDialogImplExt for T {}

unsafe impl<T: AdapDialogImpl> IsSubclassable<T> for Dialog {
    fn class_init(class: &mut glib::Class<Self>) {
        Self::parent_class_init::<T>(class);

        let klass = class.as_mut();
        klass.close_attempt = Some(close_attempt::<T>);
        klass.closed = Some(closed::<T>);
    }
}

unsafe extern "C" fn close_attempt<T: AdapDialogImpl>(ptr: *mut ffi::AdapDialog) {
    let instance = unsafe { &*(ptr as *mut T::Instance) };
    let imp = instance.imp();

    AdapDialogImpl::close_attempt(imp)
}

unsafe extern "C" fn closed<T: AdapDialogImpl>(ptr: *mut ffi::AdapDialog) {
    let instance = unsafe { &*(ptr as *mut T::Instance) };
    let imp = instance.imp();

    AdapDialogImpl::closed(imp)
}
