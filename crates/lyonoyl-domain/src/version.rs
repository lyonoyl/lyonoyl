//! Which of two versions is the later one.
//!
//! A decision and nothing else, so it lives here rather than beside the code that fetches a
//! manifest. What an updater must never do is offer a version that is not newer — that is how an
//! attacker who can influence what the manifest says serves an older build with a hole in it, and
//! how a person who declined an update is asked about it again every day for ever.
//!
//! Three numbers, and nothing after them. A version with a suffix — a pre-release, a build tag — is
//! refused rather than guessed at: this product's versions are three numbers, and a fourth thing
//! appearing means the manifest is not describing a version this build knows how to compare.

use crate::error::DomainError;

/// A version of this application, as three numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    /// Ordered first, which is what makes the derived comparison the right one.
    major: u32,
    minor: u32,
    patch: u32,
}

impl Version {
    /// Reads `1.2.3`, and refuses everything else.
    ///
    /// Leading zeroes are refused with the rest: `01.2.3` and `1.2.3` would compare equal while
    /// reading differently, and a version that can be written two ways is one an attacker can offer
    /// twice.
    pub fn parse(written: &str) -> Result<Self, DomainError> {
        let mut numbers = written.split('.');

        let mut next = || -> Result<u32, DomainError> {
            let part = numbers.next().ok_or(DomainError::InvalidVersion)?;

            if part.is_empty()
                || !part.bytes().all(|byte| byte.is_ascii_digit())
                || (part.len() > 1 && part.starts_with('0'))
            {
                return Err(DomainError::InvalidVersion);
            }

            part.parse().map_err(|_| DomainError::InvalidVersion)
        };

        let major = next()?;
        let minor = next()?;
        let patch = next()?;

        match numbers.next() {
            Some(_) => Err(DomainError::InvalidVersion),
            None => Ok(Self {
                major,
                minor,
                patch,
            }),
        }
    }

    /// Whether this one is later than the one running.
    ///
    /// The question an updater asks, phrased so the answer cannot be read backwards. Equal is not
    /// newer: a manifest that names the version already installed is a manifest saying there is
    /// nothing to do.
    pub fn is_newer_than(self, running: Self) -> bool {
        self > running
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, into: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(into, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_numbers_are_a_version_and_nothing_else_is() {
        assert!(Version::parse("1.2.3").is_ok());
        assert!(Version::parse("0.10.0").is_ok());

        for refused in [
            "1.2",
            "1.2.3.4",
            "1.2.x",
            "v1.2.3",
            "1.2.3-beta",
            "1.2.3 ",
            "",
            "..",
            "1..3",
        ] {
            assert!(
                Version::parse(refused).is_err(),
                "{refused:?} was read as a version"
            );
        }
    }

    /// A version that can be written two ways is one that can be offered twice.
    #[test]
    fn a_number_with_a_leading_zero_is_not_a_version() {
        assert!(Version::parse("01.2.3").is_err());
        assert!(Version::parse("1.02.3").is_err());
        assert!(Version::parse("1.2.03").is_err());
        // The digit alone is not a leading zero.
        assert!(Version::parse("0.0.0").is_ok());
    }

    /// The whole reason this is in the domain: an older version must never be offered.
    #[test]
    fn nothing_but_a_later_version_is_newer() {
        let running = Version::parse("1.2.3").expect("a version");

        for later in ["1.2.4", "1.3.0", "2.0.0", "1.3.3", "10.0.0"] {
            assert!(
                Version::parse(later)
                    .expect("a version")
                    .is_newer_than(running),
                "{later} was not treated as newer than 1.2.3"
            );
        }

        for not_later in ["1.2.3", "1.2.2", "1.1.9", "0.9.9", "1.0.0"] {
            assert!(
                !Version::parse(not_later)
                    .expect("a version")
                    .is_newer_than(running),
                "{not_later} was offered over 1.2.3"
            );
        }
    }

    /// Ten is after nine, which a comparison of the written form would get wrong.
    #[test]
    fn versions_are_compared_as_numbers_rather_than_as_text() {
        let nine = Version::parse("1.9.0").expect("a version");
        let ten = Version::parse("1.10.0").expect("a version");

        assert!(ten.is_newer_than(nine));
        assert!(!nine.is_newer_than(ten));
    }

    #[test]
    fn a_version_reads_back_as_it_was_written() {
        assert_eq!(
            Version::parse("1.10.0").expect("a version").to_string(),
            "1.10.0"
        );
    }
}
