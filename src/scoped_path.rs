// Source retrieved from [safe-path](https://crates.io/crates/safe-path).
// This single file has been extracted to ensure compatibility with multiple platforms.

use std::io::{Error, ErrorKind, Result};
use std::path::{Component, Path, PathBuf};

// Follow the same configuration as
// [secure_join](https://github.com/cyphar/filepath-securejoin/blob/master/join.go#L51)
const MAX_SYMLINK_DEPTH: u32 = 255;

fn do_scoped_resolve<R: AsRef<Path>, U: AsRef<Path>>(
    root: R,
    unsafe_path: U,
) -> Result<(PathBuf, PathBuf)> {
    let root = root.as_ref().canonicalize()?;

    let mut nlinks = 0u32;
    let mut curr_path = unsafe_path.as_ref().to_path_buf();
    'restart: loop {
        let mut subpath = PathBuf::new();
        let mut iter = curr_path.components();

        'next_comp: while let Some(comp) = iter.next() {
            match comp {
                // Linux paths don't have prefixes.
                Component::Prefix(_) => {
                    return Err(Error::new(
                        ErrorKind::Other,
                        format!("Invalid path prefix in: {}", unsafe_path.as_ref().display()),
                    ));
                }
                // `RootDir` should always be the first component, and Path::components() ensures
                // that.
                Component::RootDir | Component::CurDir => {
                    continue 'next_comp;
                }
                Component::ParentDir => {
                    subpath.pop();
                }
                Component::Normal(n) => {
                    let path = root.join(&subpath).join(n);
                    if let Ok(v) = path.read_link() {
                        nlinks += 1;
                        if nlinks > MAX_SYMLINK_DEPTH {
                            return Err(Error::new(
                                ErrorKind::Other,
                                format!(
                                    "Too many levels of symlinks: {}",
                                    unsafe_path.as_ref().display()
                                ),
                            ));
                        }
                        curr_path = if v.is_absolute() {
                            v.join(iter.as_path())
                        } else {
                            subpath.join(v).join(iter.as_path())
                        };
                        continue 'restart;
                    } else {
                        subpath.push(n);
                    }
                }
            }
        }

        return Ok((root, subpath));
    }
}

/// Safely join `unsafe_path` to `root`, and ensure `unsafe_path` is scoped under `root`.
///
/// The `scoped_join()` function assumes `root` exists and is an absolute path. It safely joins the
/// two given paths and ensures:
/// - The returned path is guaranteed to be scoped inside `root`.
/// - Any symbolic links in the path are evaluated with the given `root` treated as the root of the
///   filesystem, similar to a chroot.
///
/// It's modelled after [secure_join](https://github.com/cyphar/filepath-securejoin), but only
/// for Linux systems.
///
/// # Arguments
/// - `root`: the absolute path to scope the symlink evaluation.
/// - `unsafe_path`: the path to evaluated and joint with `root`. It is unsafe since it may try to
///   escape from the `root` by using "../" or symlinks.
///
/// # Security
/// On success return, the `scoped_join()` function guarantees that:
/// - The resulting PathBuf must be a child path of `root` and will not contain any symlink path
///   components (they will all get expanded).
/// - When expanding symlinks, all symlink path components must be resolved relative to the provided
///   `root`. In particular, this can be considered a userspace implementation of how chroot(2)
///    operates on file paths.
/// - Non-existent path components are unaffected.
///
/// Note that the guarantees provided by this function only apply if the path components in the
/// returned string are not modified (in other words are not replaced with symlinks on the
/// filesystem) after this function has returned. You may use [crate::PinnedPathBuf] to protect
/// from such TOCTTOU attacks.
pub fn scoped_join<R: AsRef<Path>, U: AsRef<Path>>(root: R, unsafe_path: U) -> Result<PathBuf> {
    do_scoped_resolve(root, unsafe_path).map(|(root, path)| root.join(path))
}
