// Take a look at the license at the top of the repository in the LICENSE file.

#[doc(alias = "adap_init")]
pub fn init() -> Result<(), glib::BoolError> {
    skip_assert_initialized!();
    gtk::init()?;
    unsafe {
        ffi::adap_init();
    }
    Ok(())
}
