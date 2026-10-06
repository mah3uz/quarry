#!/usr/bin/env bash
# Recreates the docs site's terminal captures (docs/.vitepress/theme/captures) from a real quarry
# session in tmux, against a small shop database it builds itself.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
quarry=${QUARRY_BIN:-$root/target/debug/quarry}
theme=$root/docs/.vitepress/theme
work=$(mktemp -d)
session=quarry-captures-$$
trap 'tmux kill-session -t "$session" 2>/dev/null || true; rm -rf "$work"' EXIT

export QUARRY_CONFIG_DIR=$work/config QUARRY_DATA_DIR=$work/data
cd "$work"

"$quarry" shop.db >/dev/null <<'SQL'
CREATE TABLE users (id integer PRIMARY KEY, email text NOT NULL UNIQUE, name text,
  created_at text DEFAULT CURRENT_TIMESTAMP, meta text CHECK (meta IS NULL OR json_valid(meta)));
CREATE TABLE products (id integer PRIMARY KEY, sku text UNIQUE, title text NOT NULL, price numeric(10,2),
  in_stock boolean DEFAULT 1);
CREATE TABLE orders (id integer PRIMARY KEY, user_id integer NOT NULL REFERENCES users(id),
  placed_at text DEFAULT CURRENT_TIMESTAMP, status text DEFAULT 'new');
CREATE TABLE order_items (order_id integer NOT NULL REFERENCES orders(id),
  product_id integer NOT NULL REFERENCES products(id), qty integer NOT NULL DEFAULT 1);
WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 500)
INSERT INTO users SELECT i, 'user' || i || '@example.com', 'User ' || i, '2026-09-30 12:01:05',
  json_object('plan', CASE WHEN i % 3 = 0 THEN 'pro' ELSE 'free' END) FROM n;
WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 50)
INSERT INTO products SELECT i, 'SKU-' || i, 'Product ' || i, round(5 + (i * 37 % 900) / 10.0, 2), 1 FROM n;
WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 2000)
INSERT INTO orders SELECT i, 1 + (i * 2654435761 % 4294967296 / 7919) % 500, '2026-09-30 12:01:05',
  CASE i % 3 WHEN 0 THEN 'new' WHEN 1 THEN 'paid' ELSE 'shipped' END FROM n;
INSERT INTO order_items SELECT id, 1 + id * 7 % 50, 1 + id % 3 FROM orders;
CREATE VIEW big_spenders AS SELECT u.name, sum(p.price * oi.qty) AS total FROM users u
  JOIN orders o ON o.user_id = u.id JOIN order_items oi ON oi.order_id = o.id
  JOIN products p ON p.id = oi.product_id GROUP BY u.name;
ANALYZE;
SQL

# A session of the given size running quarry with the given arguments, once it has drawn itself.
start() {
    local size=$1
    shift
    tmux new-session -d -s "$session" -x "${size%x*}" -y "${size#*x}" \
        -e COLORTERM=truecolor -e QUARRY_CONFIG_DIR="$QUARRY_CONFIG_DIR" -e QUARRY_DATA_DIR="$QUARRY_DATA_DIR" \
        "$quarry" --icons nerd "$@"
    sleep 2
}

# Pasted, not typed: a paste never triggers completion, so the text arrives exactly as written.
paste() {
    printf '%s' "$1" | tmux load-buffer -b quarry-capture -
    tmux paste-buffer -p -d -b quarry-capture -t "$session"
    sleep 0.5
}

# The screen with its colours, without the blank lines below the last one drawn.
save() {
    tmux capture-pane -e -p -t "$session" | python3 -c 'import sys; print(sys.stdin.read().rstrip("\n"))' >"$theme/captures/$1.ans"
    tmux kill-session -t "$session"
}

start 88x26 shop.db
paste $'select u.name, count(o.id) as orders\nfrom users u join orders o on o.user_id = u.id\ngroup by u.name order by orders desc limit 3;'
tmux send-keys -t "$session" Enter
sleep 1
tmux send-keys -t "$session" -l 'select * from orders o where o.'
sleep 1
save repl

start 104x26 shop.db --tui
paste $'select id, email, name, created_at\nfrom users\nwhere meta like \'%pro%\'\nlimit 25'
tmux send-keys -t "$session" F5
# long enough for the "Connected" notice to leave the screen
sleep 5
save tui

prompt=$'\e[38;2;158;206;106m$\e[0m'
{
    for args in '-F csv -e "select id, email from users order by id limit 3"' \
        '-F json -e "select status, count(*) as n from orders group by 1"'; do
        echo "$prompt quarry shop.db $args"
        eval "\"$quarry\" shop.db $args"
    done
} >"$theme/captures/script.ans"

# The site ships only the Nerd Font icons the captures use.
font=${NERD_FONT:-/usr/share/fonts/TTF/JetBrainsMonoNerdFontMono-Regular.ttf}
if command -v pyftsubset >/dev/null && [ -f "$font" ]; then
    cd "$theme"
    icons=$(python3 -c "import glob; print(','.join(sorted({'U+%X' % ord(c) for f in glob.glob('captures/*.ans') for c in open(f).read() if 0xE000 <= ord(c) <= 0xF8FF or ord(c) >= 0xF0000})))")
    pyftsubset "$font" --unicodes="$icons" --flavor=woff2 --layout-features='' --no-hinting \
        --output-file=fonts/NerdSymbols-subset.woff2
else
    echo "captures.sh: pyftsubset or $font is missing, so fonts/NerdSymbols-subset.woff2 was not rebuilt" >&2
    exit 1
fi
