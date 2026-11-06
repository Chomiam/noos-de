use super::dialog::AdapDialogImpl;
use crate::PreferencesDialog;
use glib::prelude::*;
use glib::subclass::prelude::*;

pub trait PreferencesDialogImpl:
    AdapDialogImpl + ObjectSubclass<Type: IsA<PreferencesDialog>>
{
}

unsafe impl<T: PreferencesDialogImpl> IsSubclassable<T> for PreferencesDialog {}
