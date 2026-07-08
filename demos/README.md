# DedrooM Demos

Three short GIF demos showing DedrooM in action:

| Demo | File | Description |
|------|------|-------------|
| Self-Healing | `demo1_self_healing.gif` | Loop detection → blocked → healing hint → agent adapts |
| Savings | `demo2_savings.gif` | Compression ratios, pipeline latency, benchmark summary |
| Quick Start | `demo3_quickstart.gif` | Install → init → use → status → stop workflow |

## Re-recording

All demos are generated from reusable shell scripts:

```bash
# Prerequisites
brew install asciinema ffmpeg
cargo install agg

# Re-record all demos
bash demos/record_all.sh
```

Each demo consists of:
- `demos/demoN_name.sh` — the shell script that drives the demo
- `demos/casts/demoN_name.cast` — raw asciinema recording
- `demos/demoN_name.gif` — final animated GIF

To re-record a single demo:

```bash
asciinema rec --overwrite --cols 90 --rows 30 \
  --command "bash demos/demo1_self_healing.sh" \
  demos/casts/demo1_self_healing.cast

agg --speed 2.0 --font-size 14 --rows 30 --cols 90 \
  demos/casts/demo1_self_healing.cast \
  demos/demo1_self_healing.gif
```

Adjust `--speed` and `--fps-cap` to control GIF playback speed and file size.
