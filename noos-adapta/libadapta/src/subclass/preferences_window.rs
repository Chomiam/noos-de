use super::window::AdapWindowImpl;
use crate::PreferencesWindow;
use glib::prelude::*;
use glib::subclass::prelude::*;

pub trait PreferencesWindowImpl:
    AdapWindowImpl + ObjectSubclass<Type: IsA<PreferencesWindow>>
{
}

unsafe impl<T: PreferencesWindowImpl> IsSubclassable<T> for PreferencesWindow {}
