# Preserve Git Bash login initialization, then repair the child-process PATH.
# A user profile can prepend Scoop shims, so this must run after the profile.
if [ -r /etc/profile ]; then
    . /etc/profile
fi
for glint_profile in "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile"; do
    if [ -r "$glint_profile" ]; then
        . "$glint_profile"
        break
    fi
done
unset glint_profile

if [ -n "${GLINT_NATIVE_GIT_BIN:-}" ]; then
    glint_git_bin=$(/usr/bin/cygpath -u -- "$GLINT_NATIVE_GIT_BIN")
    if [ -n "$glint_git_bin" ]; then
        # Native make searches PATH again for sh.exe. Repair shell lookup too,
        # rather than only git.exe, after profiles prepend package-manager shims.
        export PATH="$glint_git_bin:/usr/bin:/bin:$PATH"
        # Windows make needs a native path; /bin/bash from MSYS is not a native
        # Windows path and may cause make to fall back to the broken sh shim.
        export SHELL="$(/usr/bin/cygpath -am -- /bin/bash.exe)"
        hash -r
    fi
    unset glint_git_bin GLINT_NATIVE_GIT_BIN
fi
