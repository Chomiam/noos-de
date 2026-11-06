use crate::Application;

use glib::prelude::*;
use gtk::subclass::prelude::*;

pub trait AdapApplicationImpl: GtkApplicationImpl + ObjectSubclass<Type: IsA<Application>> {}

unsafe impl<T: AdapApplicationImpl> IsSubclassable<T> for Application {}
