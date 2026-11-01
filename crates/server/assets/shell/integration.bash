# Wraith shell integration for bash: reports the working directory (OSC 7) and prompt marks (OSC 133).
# Used as --rcfile, so it first does what an interactive shell would have done.
case "$(uname -s 2>/dev/null)" in
  MINGW*|MSYS*|CYGWIN*)
    [ -r /etc/profile ] && . /etc/profile
    [ -r "$HOME/.bash_profile" ] && . "$HOME/.bash_profile"
    ;;
esac
[ -r "$HOME/.bashrc" ] && . "$HOME/.bashrc"

if [ -z "$__WRAITH_INTEGRATED" ] && [ "$TERM_PROGRAM" = "wraith" ]; then
  __WRAITH_INTEGRATED=1
  __wraith_cwd() {
    local p
    p="$(pwd -W 2>/dev/null || pwd)"
    case "$p" in [A-Za-z]:*) p="/$p" ;; esac
    printf '%s' "${p// /%20}"
  }
  __wraith_prompt_command() {
    local ec=$?
    printf '\e]133;D;%s\e\\' "$ec"
    printf '\e]7;file://%s%s\e\\' "${HOSTNAME:-localhost}" "$(__wraith_cwd)"
    printf '\e]133;A\e\\'
  }
  PROMPT_COMMAND="__wraith_prompt_command${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
  PS1="${PS1}\[\e]133;B\e\\\\\]"
fi
