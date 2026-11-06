use crate::Window;
use glib::prelude::*;
use gtk::subclass::prelude::*;

pub trait AdapWindowImpl: WindowImpl + ObjectSubclass<Type: IsA<Window>> {}

unsafe impl<T: AdapWindowImpl> IsSubclassable<T> for Window {}
