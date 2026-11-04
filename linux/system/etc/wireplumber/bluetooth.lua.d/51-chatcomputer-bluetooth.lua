-- Chat Computer WirePlumber tuning for AirPods (and similar HFP headsets).
--
-- Target: WirePlumber 0.4.x on Ubuntu 24.04 (Lua config). It runs the files of
-- bluetooth.lua.d in /usr/share/wireplumber and /etc/wireplumber together, in
-- name order: this one after the stock 50-bluez-config.lua (so its settings
-- win) and before 90-enable-all.lua (which starts the monitor with them).
-- We touch two things:
--
--   1. bluez monitor properties: force-enable wideband speech (mSBC) and the
--      SBC-XQ / hardware-volume features so the AirPods mic sounds usable and
--      the volume keys work. These correspond to bluez5.enable-* in the stock
--      30-bluez-monitor.lua (commented out there = "use hardware quirk DB").
--   2. HFP backend = native (no oFono/ModemManager needed).
--
-- The actual "switch to the headset/mic profile when the mic opens"
-- auto-switch is in 51-chatcomputer-policy.lua (bluetooth_policy).

bluez_monitor = bluez_monitor or {}
bluez_monitor.properties = bluez_monitor.properties or {}

-- Enable Hands-Free Profile (so the AirPods microphone is available at all) and
-- A2DP for high-quality playback. hsp_* left off: like the stock comment notes,
-- some headsets misbehave with both HSP and HFP AG enabled; HFP is what modern
-- AirPods use.
bluez_monitor.properties["bluez5.roles"] =
  "[ a2dp_sink a2dp_source bap_sink bap_source hfp_hf hfp_ag ]"

-- Wideband speech (mSBC): much better mic quality than the 8 kHz CVSD fallback.
bluez_monitor.properties["bluez5.enable-msbc"] = true
-- SBC-XQ: higher-quality A2DP playback where the device supports it.
bluez_monitor.properties["bluez5.enable-sbc-xq"] = true
-- Let the headset's own volume keys drive the system volume.
bluez_monitor.properties["bluez5.enable-hw-volume"] = true
-- Native HFP backend: no oFono / ModemManager dependency.
bluez_monitor.properties["bluez5.hfphsp-backend"] = "native"

-- Reconnect profiles automatically when the AirPods come back, and prefer
-- starting on A2DP (music) — the policy script promotes to the mic profile
-- on demand.
bluez_monitor.rules = bluez_monitor.rules or {}
table.insert(bluez_monitor.rules, {
  matches = {
    { { "device.name", "matches", "bluez_card.*" } },
  },
  apply_properties = {
    ["bluez5.auto-connect"]  = "[ hfp_hf a2dp_sink ]",
    ["bluez5.hw-volume"]     = "[ hfp_hf a2dp_sink ]",
    ["device.profile"]       = "a2dp-sink",
  },
})
