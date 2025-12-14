@echo off
cargo build --release
copy ".\target\release\starlight_shards_as_rune_arcs.dll" "C:\Program Files (x86)\Steam\steamapps\common\ELDEN RING\Game\mod_dll\starlight-shards-as-rune-arcs\"
