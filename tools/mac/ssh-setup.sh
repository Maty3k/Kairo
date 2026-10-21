#!/bin/bash
# SPDX-License-Identifier: GPL-3.0-or-later
# tools/mac/ssh-setup.sh: the Mac side of Kairo's SSH bridge (docs/PLAN.md section 3.1, M0).
#
#   tools/mac/ssh-setup.sh                                   1. key only (before the bootstrap)
#   tools/mac/ssh-setup.sh <pc-ip> <windows-user>            2. also the ssh config block
#   tools/mac/ssh-setup.sh <pc-ip> <windows-user> <SHA256:fingerprint>
#                                                            3. also pin the PC's host key
#   tools/mac/ssh-setup.sh <pc-ip> <windows-user> --trust-on-first-use
#                                                            3. pin it unchecked; prints it to compare
#
# 1. Creates ~/.ssh/kairo_pc (ed25519) unless it already exists; it never overwrites a key. The
#    key is encrypted with a random passphrase that nobody sees: it goes straight into a login
#    Keychain item ("Kairo SSH key"), which tools/mac/keychain-askpass reads whenever the key has
#    to be loaded into ssh-agent, and the script loads the key that way before it keeps it.
# 2. Writes or updates one "Host kairo-pc" block in ~/.ssh/config between marker lines, so
#    running it again (for example with a new IP) never adds a second block.
# 3. Compares the host key the PC presents with the fingerprint the bootstrap showed on the PC
#    (the "Host key: SHA256:..." line of its DONE screen) and, if they match, adds it to
#    ~/.ssh/known_hosts. "tools/mac/pc" uses BatchMode and StrictHostKeyChecking=yes, so it never
#    accepts an unknown or changed host key by itself. On the PC (PowerShell) the same value is:
#      ssh-keygen -lf $env:ProgramData\ssh\ssh_host_ed25519_key.pub
#    --trust-on-first-use pins whatever key answers and prints its fingerprint; use it only when
#    the DONE screen's line is missing, on the home network, right after the bootstrap, and have
#    the owner compare it afterwards.
# Always ends by printing the public key, for the bootstrap one-liner (-MacKey '<public key>').
#
# The key is a standing elevated shell on the PC (the owner is an administrator), which is why
# its passphrase lives in the Keychain. Nothing here talks to the internet; step 3 talks to the PC.
# Written for macOS's /bin/bash 3.2.

set -u
set -o pipefail
export LC_ALL=C

KEY=${KAIRO_PC_KEY:-$HOME/.ssh/kairo_pc}
SELF_DIR=$(cd "$(dirname "$0")" && pwd -P) || exit 1
ASKPASS_HELPER="$SELF_DIR/keychain-askpass"
KEYCHAIN_SERVICE='Kairo SSH key'   # tools/mac/keychain-askpass reads this item
ALIAS=kairo-pc
SSH_DIR="$HOME/.ssh"
CONFIG="$SSH_DIR/config"
KNOWN_HOSTS="$SSH_DIR/known_hosts"
BEGIN_MARK='# >>> kairo-pc: written by Kairo tools/mac/ssh-setup.sh; run it again to change >>>'
END_MARK='# <<< kairo-pc <<<'
TMP_BASE=${TMPDIR:-/tmp}
TMP_BASE=${TMP_BASE%/}

say() { printf '%s\n' "$*"; }
die() {
    printf 'ssh-setup: %s\n' "$1" >&2
    exit "${2:-1}"
}
usage() {
    sed -n '5,10p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
}

WORK=''
# shellcheck disable=SC2317,SC2329 # called by the EXIT trap below
cleanup() { [ -n "$WORK" ] && rm -rf "$WORK"; }
trap cleanup EXIT

[ "$(uname -s)" = Darwin ] || die "this script is for macOS (it uses the macOS Keychain)"
for tool in /usr/bin/ssh-keygen /usr/bin/ssh-add /usr/bin/ssh-keyscan /usr/bin/base64 /usr/bin/security "$ASKPASS_HELPER"; do
    [ -x "$tool" ] || die "missing $tool"
done
# The key's path is the Keychain item's account name, written inside "security -i" quotes.
case "$KEY" in
/*) ;;
*) die "KAIRO_PC_KEY must be an absolute path" 2 ;;
esac
case "$KEY" in *\"* | *\\* | *[![:print:]]*) die "unexpected characters in the key path: $KEY" 2 ;; esac
case $# in
0 | 2 | 3) ;;
*) usage ;;
esac
case "${1:-}" in -h | --help) usage ;; esac

# Check every argument before changing anything.
host=${1:-}
user=${2:-}
user=${user##*\\}   # the bare account name: "PC\name" or "DOMAIN\name" -> "name"
want=''
tofu=0
if [ $# -ge 2 ]; then
    case "$host" in '' | -* | *[!A-Za-z0-9.:-]*) die "not an IP address or host name: $host" 2 ;; esac
    [ "${#host}" -le 253 ] || die "host name too long" 2
    case "$user" in '' | -* | ' '* | *' ' | *[!A-Za-z0-9._@\ -]*) die "unexpected Windows user name: $user (letters, digits, space . _ @ -)" 2 ;; esac
    [ "${#user}" -le 64 ] || die "Windows user name too long" 2
fi
if [ $# -ge 3 ]; then
    if [ "$3" = --trust-on-first-use ]; then
        tofu=1
    else
        want=${3#SHA256:}
        case "$want" in '' | *[!A-Za-z0-9+/=]*) die "the fingerprint should look like SHA256:abc...xyz, or use --trust-on-first-use" 2 ;; esac
    fi
fi

# ------------------------------------------------------------------------------------------
# 1. The key
# ------------------------------------------------------------------------------------------
mkdir -p "$SSH_DIR" || die "cannot create $SSH_DIR"
chmod 700 "$SSH_DIR" || die "cannot set the permissions of $SSH_DIR"

make_key() {
    local secret
    WORK=$(mktemp -d "$TMP_BASE/kairo-ssh.XXXXXX") || die "mktemp failed"
    chmod 700 "$WORK"
    # The askpass helper answers at most KAIRO_ASKPASS_MAX times, then returns nothing: ssh-add
    # would otherwise ask again forever if a passphrase were ever wrong.
    cat >"$WORK/askpass" <<'EOF'
#!/bin/sh
count=0
[ -f "$KAIRO_ASKPASS_COUNT" ] && count=$(cat "$KAIRO_ASKPASS_COUNT")
count=$((count + 1))
printf '%s' "$count" > "$KAIRO_ASKPASS_COUNT"
[ "$count" -le "${KAIRO_ASKPASS_MAX:-1}" ] || exit 1
printf '%s\n' "$KAIRO_ASKPASS_SECRET"
EOF
    chmod 700 "$WORK/askpass"
    secret=$(head -c 33 /dev/urandom | /usr/bin/base64 | tr -d '\n')
    [ "${#secret}" -ge 40 ] || die "could not make a random passphrase"

    say "creating $KEY (ed25519, random passphrase kept only in the macOS Keychain)"
    if ! KAIRO_ASKPASS_SECRET="$secret" KAIRO_ASKPASS_COUNT="$WORK/count-keygen" KAIRO_ASKPASS_MAX=2 \
        SSH_ASKPASS="$WORK/askpass" SSH_ASKPASS_REQUIRE=force \
        /usr/bin/ssh-keygen -q -t ed25519 -f "$KEY" -C kairo-mac </dev/null; then
        rm -f "$KEY" "$KEY.pub"
        die "ssh-keygen failed"
    fi
    # "security -i" reads the command on stdin (printf is a bash builtin), so the passphrase is
    # never on a command line that other programs could read. -U replaces an item from an
    # earlier attempt.
    printf 'add-generic-password -U -s "%s" -a "%s" -l "%s (%s)" -w "%s"\n' \
        "$KEYCHAIN_SERVICE" "$KEY" "$KEYCHAIN_SERVICE" "$KEY" "$secret" | /usr/bin/security -i >/dev/null 2>&1
    secret=''
    # The proof that the Keychain gives the passphrase back: load the key the way tools/mac/pc does.
    if ! load_key; then
        /usr/bin/security delete-generic-password -s "$KEYCHAIN_SERVICE" -a "$KEY" >/dev/null 2>&1
        rm -f "$KEY" "$KEY.pub"
        die "could not load the new key with its passphrase from the Keychain (is the login Keychain unlocked?); removed the key, which nobody could unlock"
    fi
    say "the key is in ssh-agent and its passphrase is in the login Keychain (item \"$KEYCHAIN_SERVICE\")"
}

# Loads $KEY into ssh-agent; tools/mac/keychain-askpass answers ssh-add's passphrase prompt once.
load_key() {
    local stamp status
    stamp=$(mktemp "$TMP_BASE/kairo-askpass.XXXXXX") || return 1
    KAIRO_PC_KEY="$KEY" KAIRO_ASKPASS_STAMP="$stamp" SSH_ASKPASS="$ASKPASS_HELPER" SSH_ASKPASS_REQUIRE=force \
        /usr/bin/ssh-add -q "$KEY" </dev/null >/dev/null 2>&1
    status=$?
    rm -f "$stamp"
    return "$status"
}

if [ -e "$KEY" ] || [ -e "$KEY.pub" ]; then
    if [ ! -f "$KEY" ] || [ ! -f "$KEY.pub" ]; then
        die "found only half of the key pair ($KEY, $KEY.pub); not touching it. Fix or move it by hand."
    fi
    say "$KEY already exists; keeping it"
    load_key || say "note: could not load $KEY into ssh-agent with its passphrase from the Keychain"
else
    make_key
fi

# ------------------------------------------------------------------------------------------
# 2. The ssh config block
# ------------------------------------------------------------------------------------------
if [ $# -ge 2 ]; then
    user_value=$user
    case "$user" in *' '*) user_value="\"$user\"" ;; esac
    identity=$KEY
    # shellcheck disable=SC2088 # a literal ~ on purpose: ssh expands it in ~/.ssh/config
    case "$KEY" in "$HOME/.ssh/"*) identity="~/.ssh/${KEY#"$HOME/.ssh/"}" ;; esac

    WORK=${WORK:-$(mktemp -d "$TMP_BASE/kairo-ssh.XXXXXX")} || die "mktemp failed"
    block="$WORK/block"
    cat >"$block" <<EOF
$BEGIN_MARK
Host $ALIAS
    HostName $host
    User $user_value
    IdentityFile $identity
    IdentitiesOnly yes
    StrictHostKeyChecking yes
    ServerAliveInterval 30
$END_MARK
EOF
    touch "$CONFIG" || die "cannot write $CONFIG"
    new_config="$SSH_DIR/.config.kairo-$$"
    if grep -qF "$BEGIN_MARK" "$CONFIG"; then
        grep -qF "$END_MARK" "$CONFIG" || die "the kairo-pc block in $CONFIG has lost its end line ($END_MARK); fix it by hand"
        # Replace the block this script wrote last time, in place.
        awk -v begin="$BEGIN_MARK" -v end="$END_MARK" -v block="$block" '
            $0 == begin { while ((getline line < block) > 0) print line; skip = 1; next }
            skip && $0 == end { skip = 0; next }
            !skip { print }' "$CONFIG" >"$new_config" || die "could not rewrite $CONFIG"
        action=updated
    elif grep -Eiq "^[[:space:]]*Host[[:space:]]+(.*[[:space:]])?$ALIAS([[:space:]]|\$)" "$CONFIG"; then
        die "$CONFIG already has a 'Host $ALIAS' entry that this script did not write; edit it by hand (HostName $host, User $user, IdentityFile $identity)"
    else
        {
            cat "$CONFIG"
            if [ -s "$CONFIG" ] && [ "$(tail -c 1 "$CONFIG" | od -An -c | tr -d ' ')" != '\n' ]; then echo; fi
            [ -s "$CONFIG" ] && echo
            cat "$block"
        } >"$new_config" || die "could not write $new_config"
        action=added
    fi
    if [ -L "$CONFIG" ]; then
        # A symlinked config (dotfiles repo): write through the link instead of replacing it.
        if ! cat "$new_config" >"$CONFIG"; then die "could not write $CONFIG"; fi
        rm -f "$new_config"
    else
        if ! chmod 600 "$new_config"; then die "could not set the permissions of $new_config"; fi
        if ! mv "$new_config" "$CONFIG"; then die "could not replace $CONFIG"; fi
    fi
    say "$action the '$ALIAS' block in $CONFIG (HostName $host, User $user)"
fi

# ------------------------------------------------------------------------------------------
# 3. The PC's host key
# ------------------------------------------------------------------------------------------
if [ $# -ge 3 ]; then
    scanned=$(/usr/bin/ssh-keyscan -T 5 -t ed25519 "$host" 2>/dev/null | grep -v '^#' | head -n 1)
    [ -n "$scanned" ] || die "could not read the host key of $host (is the PC on, and is sshd running?)"
    got=$(printf '%s\n' "$scanned" | /usr/bin/ssh-keygen -lf - 2>/dev/null | awk '{print $2}')
    got=${got#SHA256:}
    [ -n "$got" ] || die "could not compute the fingerprint of the key $host presented"
    if [ "$tofu" = 1 ]; then
        say "trusting the key $host presents now: SHA256:$got"
        say "ask the owner to compare it with the 'Host key' line of the bootstrap's DONE screen, or on the PC:"
        say "  ssh-keygen -lf \$env:ProgramData\\ssh\\ssh_host_ed25519_key.pub"
    elif [ "$got" != "$want" ]; then
        die "HOST KEY MISMATCH: $host presents SHA256:$got, but the PC showed SHA256:$want. Not trusting it; do not connect, and tell the owner."
    fi
    key_part=${scanned#* }
    touch "$KNOWN_HOSTS" || die "cannot write $KNOWN_HOSTS"
    known=$(/usr/bin/ssh-keygen -F "$host" -f "$KNOWN_HOSTS" 2>/dev/null | grep -v '^#' | grep ' ssh-ed25519 ')
    if [ -n "$known" ]; then
        case "$known" in
        *"$key_part"*) say "the host key of $host is already pinned in $KNOWN_HOSTS" ;;
        *) die "$KNOWN_HOSTS has a different ed25519 key for $host (an old device at this address?). Check, then remove it with: ssh-keygen -R $host" ;;
        esac
    else
        printf '%s\n' "$scanned" >>"$KNOWN_HOSTS" || die "could not append to $KNOWN_HOSTS"
        say "pinned the host key of $host (SHA256:$got) in $KNOWN_HOSTS"
    fi
fi

# ------------------------------------------------------------------------------------------
# The public key, for the bootstrap one-liner (-MacKey '<public key>')
# ------------------------------------------------------------------------------------------
say ""
say "public key for the bootstrap (-MacKey '...'):"
cat "$KEY.pub"
