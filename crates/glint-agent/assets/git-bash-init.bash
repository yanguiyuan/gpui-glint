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
        export PATH="$glint_git_bin:$PATH"
        hash -r
    fi
    unset glint_git_bin GLINT_NATIVE_GIT_BIN
fi
