# Wraith shell integration for zsh (loaded through ZDOTDIR): OSC 7 (cwd) and OSC 133 (prompt marks).
export ZDOTDIR="${WRAITH_ORIG_ZDOTDIR:-$HOME}"
[ -r "$ZDOTDIR/.zshrc" ] && source "$ZDOTDIR/.zshrc"

if [[ -z "$__WRAITH_INTEGRATED" && "$TERM_PROGRAM" == "wraith" ]]; then
  __WRAITH_INTEGRATED=1
  autoload -Uz add-zsh-hook
  __wraith_precmd() {
    local ec=$?
    printf '\e]133;D;%s\e\\' "$ec"
    printf '\e]7;file://%s%s\e\\' "${HOST:-localhost}" "${PWD// /%20}"
    printf '\e]133;A\e\\'
  }
  __wraith_preexec() { printf '\e]133;C\e\\' }
  add-zsh-hook precmd __wraith_precmd
  add-zsh-hook preexec __wraith_preexec
  PS1="$PS1%{"$'\e]133;B\e\\'"%}"
fi
