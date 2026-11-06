use crate::ApplicationWindow;
use glib::prelude::*;
use gtk::subclass::prelude::*;

pub trait AdapApplicationWindowImpl:
    ApplicationWindowImpl + ObjectSubclass<Type: IsA<ApplicationWindow>>
{
}

unsafe impl<T: AdapApplicationWindowImpl> IsSubclassable<T> for ApplicationWindow {}
