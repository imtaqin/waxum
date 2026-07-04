#!/usr/bin/env bash
# wa-rs install / update / uninstall script.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/fdciabdul/wa-rs/main/scripts/install.sh | sudo bash
#   sudo ./install.sh install     # first-time install
#   sudo ./install.sh update      # pull latest release + restart
#   sudo ./install.sh uninstall   # remove service + binary (data kept)
#
# Fetches the latest release binary from
# https://github.com/fdciabdul/wa-rs/releases and installs it as a
# systemd service that auto-restarts on failure. On first install it
# asks whether to enable a nightly cron that self-updates.

set -euo pipefail

readonly REPO="fdciabdul/wa-rs"
readonly BIN_DIR="/usr/local/bin"
readonly BIN_PATH="${BIN_DIR}/wa-rs"
readonly DATA_DIR="/var/lib/wa-rs"
readonly ENV_FILE="/etc/wa-rs.env"
readonly SERVICE_FILE="/etc/systemd/system/wa-rs.service"
readonly CRON_FILE="/etc/cron.d/wa-rs-update"
readonly UPDATE_HELPER="/usr/local/sbin/wa-rs-update"
readonly VERSION_FILE="${DATA_DIR}/.version"

CYAN='\033[0;36m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
DIM='\033[2m'
RESET='\033[0m'

banner() {
    cat <<'ART'

██╗    ██╗ █████╗       ██████╗ ███████╗
██║    ██║██╔══██╗      ██╔══██╗██╔════╝
██║ █╗ ██║███████║█████╗██████╔╝███████╗
██║███╗██║██╔══██║╚════╝██╔══██╗╚════██║
╚███╔███╔╝██║  ██║      ██║  ██║███████║
 ╚══╝╚══╝ ╚═╝  ╚═╝      ╚═╝  ╚═╝╚══════╝

  WhatsApp Gateway REST API — installer
  https://github.com/fdciabdul/wa-rs

ART
}

log()  { echo -e "${CYAN}[wa-rs]${RESET} $*"; }
ok()   { echo -e "${GREEN}[wa-rs]${RESET} $*"; }
warn() { echo -e "${YELLOW}[wa-rs]${RESET} $*"; }
die()  { echo -e "${RED}[wa-rs]${RESET} $*" >&2; exit 1; }

need_root() {
    if [[ ${EUID} -ne 0 ]]; then
        die "must run as root — try: sudo $0 ${1:-install}"
    fi
}

require() {
    for c in "$@"; do
        command -v "$c" >/dev/null 2>&1 || die "missing required command: $c"
    done
}

detect_arch() {
    local m
    m="$(uname -m)"
    case "$m" in
        x86_64|amd64)   echo "linux-amd64" ;;
        aarch64|arm64)  echo "linux-arm64" ;;
        *)              die "unsupported architecture: $m" ;;
    esac
}

fetch_latest_tag() {
    curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep -m1 '"tag_name"' \
        | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/'
}

download_binary() {
    local tag="$1" arch="$2"
    local version="${tag#v}"
    local url="https://github.com/${REPO}/releases/download/${tag}/wa-rs-${version}-${arch}.tar.gz"
    local tmp
    tmp="$(mktemp -d)"
    trap "rm -rf '$tmp'" RETURN

    log "downloading ${tag} (${arch})…"
    curl -fsSL "$url" -o "${tmp}/wa-rs.tar.gz" || die "download failed: $url"
    tar -xzf "${tmp}/wa-rs.tar.gz" -C "$tmp"
    [[ -f "${tmp}/wa-rs" ]] || die "release archive missing wa-rs binary"

    install -Dm755 "${tmp}/wa-rs" "$BIN_PATH"
    echo "$tag" > "$VERSION_FILE"
    ok "installed binary at ${BIN_PATH}"
}

ensure_dirs() {
    install -d -m 0750 "$DATA_DIR" "${DATA_DIR}/whatsapp_sessions"
}

write_env_if_absent() {
    if [[ -f "$ENV_FILE" ]]; then
        log "env file exists at ${ENV_FILE} — leaving as-is"
        return
    fi
    umask 077
    cat > "$ENV_FILE" <<EOF
# wa-rs environment. Edit and then: systemctl restart wa-rs
DATABASE_URL=sqlite://${DATA_DIR}/wa-rs.db
JWT_SECRET=$(openssl rand -base64 48 | tr -d '\n=/+')
SUPERADMIN_TOKEN=$(openssl rand -hex 24)
WHATSAPP_STORAGE_PATH=${DATA_DIR}/whatsapp_sessions
RUST_LOG=wa_rs=info,tower_http=info
WA_RS_BLOCKING_THREADS=256
WA_RS_MYSQL_MAX_POOL=64
NATS_URL=
EOF
    chmod 600 "$ENV_FILE"
    ok "created ${ENV_FILE} — review before starting"
}

write_service() {
    cat > "$SERVICE_FILE" <<EOF
[Unit]
Description=wa-rs WhatsApp Gateway REST API
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
EnvironmentFile=${ENV_FILE}
ExecStart=${BIN_PATH}
WorkingDirectory=${DATA_DIR}
Restart=always
RestartSec=5
LimitNOFILE=1048576
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=${DATA_DIR}

[Install]
WantedBy=multi-user.target
EOF
    systemctl daemon-reload
    ok "wrote systemd unit at ${SERVICE_FILE}"
}

write_update_helper() {
    cat > "$UPDATE_HELPER" <<'EOF'
#!/usr/bin/env bash
# Self-update helper installed by wa-rs installer. Downloads the latest
# release, compares against the installed version, and restarts the
# service only if there's a new tag.
set -euo pipefail

REPO="fdciabdul/wa-rs"
BIN_PATH="/usr/local/bin/wa-rs"
VERSION_FILE="/var/lib/wa-rs/.version"

case "$(uname -m)" in
    x86_64|amd64)  ARCH="linux-amd64" ;;
    aarch64|arm64) ARCH="linux-arm64" ;;
    *) echo "unsupported arch" >&2; exit 1 ;;
esac

TAG=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep -m1 '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')
CURRENT=$(cat "$VERSION_FILE" 2>/dev/null || echo "")
if [[ "$TAG" == "$CURRENT" ]]; then
    echo "wa-rs already at ${TAG}"
    exit 0
fi

VERSION="${TAG#v}"
URL="https://github.com/${REPO}/releases/download/${TAG}/wa-rs-${VERSION}-${ARCH}.tar.gz"
TMP=$(mktemp -d)
trap "rm -rf $TMP" EXIT
curl -fsSL "$URL" -o "${TMP}/wa-rs.tar.gz"
tar -xzf "${TMP}/wa-rs.tar.gz" -C "$TMP"
install -Dm755 "${TMP}/wa-rs" "$BIN_PATH"
echo "$TAG" > "$VERSION_FILE"
systemctl restart wa-rs
echo "wa-rs updated ${CURRENT:-fresh} → ${TAG}"
EOF
    chmod +x "$UPDATE_HELPER"
    ok "installed update helper at ${UPDATE_HELPER}"
}

ask_yesno() {
    local q="$1" default="${2:-n}" ans
    local suffix
    if [[ "$default" == "y" ]]; then suffix="[Y/n]"; else suffix="[y/N]"; fi
    # `curl | sudo bash` closes stdin so `read` would EOF and, with
    # `set -e`, kill the installer. When there's no TTY we silently
    # accept the default. Overrides available via env:
    #   WA_RS_YES=1                  → accept every prompt
    #   WA_RS_AUTO_UPDATE=y/n        → answer auto-update prompt
    #   WA_RS_START=y/n              → answer "start now?"
    #   WA_RS_UNINSTALL_CONFIRM=y/n  → answer uninstall confirm
    if [[ "${WA_RS_YES:-}" == "1" ]]; then
        [[ "$default" == "y" ]] || return 1
        return 0
    fi
    case "$q" in
        *auto-update*)  [[ -n "${WA_RS_AUTO_UPDATE:-}" ]] && ans="${WA_RS_AUTO_UPDATE}" ;;
        *start*)        [[ -n "${WA_RS_START:-}" ]]       && ans="${WA_RS_START}" ;;
        *remove*|*uninstall*) [[ -n "${WA_RS_UNINSTALL_CONFIRM:-}" ]] && ans="${WA_RS_UNINSTALL_CONFIRM}" ;;
    esac
    if [[ -z "${ans:-}" ]]; then
        if [[ -t 0 ]]; then
            read -r -p "$(echo -e "${CYAN}?${RESET} ${q} ${suffix} ")" ans || ans=""
        else
            warn "no TTY — using default '$default' for: $q"
            ans="$default"
        fi
    fi
    ans="${ans:-$default}"
    [[ "${ans,,}" == "y" ]]
}

setup_cron() {
    if ask_yesno "enable nightly auto-update cron (03:15 daily)?" "y"; then
        cat > "$CRON_FILE" <<EOF
15 3 * * * root ${UPDATE_HELPER} >>/var/log/wa-rs-update.log 2>&1
EOF
        chmod 644 "$CRON_FILE"
        ok "auto-update cron enabled — logs at /var/log/wa-rs-update.log"
    else
        rm -f "$CRON_FILE"
        log "auto-update cron disabled — run '${UPDATE_HELPER}' manually to upgrade"
    fi
}

do_install() {
    banner
    need_root install
    require curl tar systemctl openssl grep sed
    local arch tag
    arch="$(detect_arch)"
    tag="$(fetch_latest_tag)"
    [[ -n "$tag" ]] || die "could not resolve latest release tag from GitHub"
    log "latest release: ${tag}"

    ensure_dirs
    download_binary "$tag" "$arch"
    write_env_if_absent
    write_service
    write_update_helper
    setup_cron

    if ask_yesno "start wa-rs now?" "y"; then
        systemctl enable --now wa-rs
        sleep 2
        systemctl --no-pager status wa-rs || true
    else
        warn "not started — enable later with: systemctl enable --now wa-rs"
    fi

    print_summary "$tag"
}

# Read the value of an env key from $ENV_FILE.
env_val() {
    local key="$1"
    grep -E "^${key}=" "$ENV_FILE" 2>/dev/null | head -n1 | sed -E "s/^${key}=//"
}

# Best-effort guess for the public host so the summary URL is useful. Falls
# back to `localhost` when nothing better is discoverable.
detect_host() {
    local h
    h="$(hostname -I 2>/dev/null | awk '{print $1}')"
    if [[ -z "$h" ]]; then
        h="$(hostname -f 2>/dev/null || hostname)"
    fi
    [[ -n "$h" ]] || h="localhost"
    echo "$h"
}

print_summary() {
    local tag="$1"
    local token url_local url_public host
    token="$(env_val SUPERADMIN_TOKEN)"
    host="$(detect_host)"
    url_local="http://127.0.0.1:3451"
    url_public="http://${host}:3451"

    echo ""
    echo -e "${GREEN}╔══════════════════════════════════════════════════════════════════╗${RESET}"
    echo -e "${GREEN}║${RESET}  ${CYAN}wa-rs installed — save this info${RESET}                              ${GREEN}║${RESET}"
    echo -e "${GREEN}╚══════════════════════════════════════════════════════════════════╝${RESET}"
    echo ""
    echo -e "  ${DIM}version${RESET}         : ${tag}"
    echo -e "  ${DIM}binary${RESET}          : ${BIN_PATH}"
    echo -e "  ${DIM}env file${RESET}        : ${ENV_FILE}"
    echo -e "  ${DIM}data dir${RESET}        : ${DATA_DIR}"
    echo -e "  ${DIM}service${RESET}         : ${SERVICE_FILE}"
    echo -e "  ${DIM}update helper${RESET}   : ${UPDATE_HELPER}"
    if [[ -f "$CRON_FILE" ]]; then
        echo -e "  ${DIM}auto-update${RESET}     : ${GREEN}nightly 03:15${RESET} (${CRON_FILE})"
    else
        echo -e "  ${DIM}auto-update${RESET}     : ${YELLOW}disabled${RESET} — run '${UPDATE_HELPER}' to upgrade manually"
    fi
    echo ""
    echo -e "  ${CYAN}Endpoints${RESET}"
    echo -e "    local           : ${url_local}"
    echo -e "    LAN             : ${url_public}"
    echo -e "    health          : ${url_local}/health"
    echo -e "    swagger UI      : ${url_local}/swagger-ui"
    echo ""
    echo -e "  ${CYAN}Credentials${RESET} ${YELLOW}(auto-generated — treat as secret)${RESET}"
    echo -e "    SUPERADMIN_TOKEN: ${token:-<missing — check ${ENV_FILE}>}"
    echo ""
    echo -e "  ${CYAN}Smoke test${RESET}"
    echo -e "    curl -s ${url_local}/health"
    echo -e "    curl -s -H 'Authorization: Bearer ${token}' ${url_local}/api/v1/sessions"
    echo ""
    echo -e "  ${DIM}logs:${RESET}    journalctl -fu wa-rs"
    echo -e "  ${DIM}restart:${RESET} systemctl restart wa-rs"
    echo ""
}

do_update() {
    banner
    need_root update
    require curl tar systemctl grep sed
    "$UPDATE_HELPER"
}

do_uninstall() {
    banner
    need_root uninstall
    if ask_yesno "stop and remove wa-rs service? (data in ${DATA_DIR} kept)" "n"; then
        systemctl disable --now wa-rs || true
        rm -f "$SERVICE_FILE" "$CRON_FILE" "$UPDATE_HELPER" "$BIN_PATH"
        systemctl daemon-reload
        ok "removed service, binary, cron. ${DATA_DIR} and ${ENV_FILE} left in place."
        warn "delete manually if you're done: rm -rf ${DATA_DIR} ${ENV_FILE}"
    else
        log "aborted"
    fi
}

main() {
    local cmd="${1:-install}"
    case "$cmd" in
        install)    do_install ;;
        update)     do_update ;;
        uninstall)  do_uninstall ;;
        --help|-h|help)
            banner
            echo "Usage: $0 [install|update|uninstall]"
            ;;
        *)
            die "unknown subcommand: $cmd (try install|update|uninstall)"
            ;;
    esac
}

main "$@"
