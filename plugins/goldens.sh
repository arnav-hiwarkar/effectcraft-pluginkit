#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/lib.sh"
OUT="${1:-$APP_TARGET/plugin-goldens}"
cd "$ROOT"
rm -rf "$OUT"
mkdir -p "$OUT"

stored_frames=()
golden() {
    local plugin="$1" name="$2"
    shift 2
    stored_frames+=("plugins/$plugin/tests/frames/$name.png")
    xtask_plugin render "plugins/$plugin" "$@" --compare "plugins/$plugin/tests/frames/$name.png" --out "$OUT/golden/$plugin-$name.png" > /dev/null
    echo "PASS $plugin $name: the kit test harness render matches its stored frame"
}

golden cookbook tint --effect tint --set amount=100
golden cookbook blur-checker --effect blur --pattern checker --size 160x90 --set radius=6
golden cookbook glow --effect glow --set radius=24
golden cookbook edges --effect edges --set width=3
golden curves s-curve-red-lift --effect curves \
    --set 'curves={"channels":[[[0,0],[0.3,0.2],[0.7,0.8],[1,1]],[[0,0.1],[1,1]],[[0,0],[1,1]],[[0,0],[1,0.8]],[[0,0],[1,1]]]}'
golden ember fountain --effect ember --time 1 --set Physics/turbulence=300
golden ember sparks --effect ember --time 0.8 --set Emitter/type=1 --set Emitter/emission=1 --set Emitter/rate=4000 --set Particle/shape=2
golden forge-3d layer-only --effect forge
golden generate ramp-linear --effect ramp --size 160x90
golden generate ramp-radial --effect ramp --size 160x90 --set shape=1
golden generate field --effect field --size 160x90 --set scale=24
golden generate dots --effect dots --size 160x90 --set columns=3 --set rows=2 --set spacing=40 --set radius=12
golden halo sun --effect flare --set look=1
golden halo anamorphic --effect flare --set look=2 --set light=200,60 --set Streak/size=150
golden plasma-beam default --effect beam --set preset=0
golden plasma-beam electric-distorted --effect beam --time 1.25 --set preset=2 --set Distortion/amount=250
golden retro crt-default --effect crt --set Screen/curvature=0.1 --set Signal/noise=0
golden retro crt-slot-wide-glow --effect crt --set Mask/mask=1 --set 'Phosphor Glow/glow_radius=45' --set Signal/noise=0
golden retro neon --effect neon --set Bloom/intensity=3

for stored in plugins/*/tests/frames/*.png; do
    case " ${stored_frames[*]} " in
        *" $stored "*) ;;
        *)
            echo "FAIL $stored has no golden line in plugins/goldens.sh"
            exit 1
            ;;
    esac
done
