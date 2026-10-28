/// Whether the person is looking at this window right now.
///
/// A port for one boolean, because the answer comes from the windowing system and the thing that
/// asks it must not know that. It is asked before any interruption: telling somebody that mail
/// arrived, while they are reading that mail, is the application talking over itself.
pub trait Attention: Send + Sync {
    /// True while the window has the focus of the system.
    ///
    /// A device that cannot answer says **yes**, and deliberately: unsure means silent. Being told
    /// nothing is a smaller failure than an interruption nobody could have wanted, and a machine
    /// whose windowing system will not say is a machine where the answer is most likely yes anyway
    /// — there is one window and it is open.
    fn has_it(&self) -> bool;
}
