-- Chat Computer WirePlumber policy: auto-switch Bluetooth to the headset (mic)
-- profile whenever an input stream opens, and back to A2DP when it closes.
--
-- This overlays bluetooth_policy from the stock policy.lua.d/10-default-policy.lua
-- (it runs after it, and before 90-enable-all.lua starts the policy with it).
-- policy-bluetooth.lua switches to a profile with an input route when it sees a
-- stream whose media.role is "Communication" OR whose application.name is in
-- media-role.applications. Chat Computer's streams are both (chatcomputer/voice/audio.py
-- names them through PIPEWIRE_ALSA), so the AirPods flip to HFP automatically
-- when Chat Computer starts listening.

bluetooth_policy = bluetooth_policy or {}
bluetooth_policy.policy = bluetooth_policy.policy or {}

-- Switch to the headset profile in the presence of an input stream.
bluetooth_policy.policy["media-role.use-headset-profile"] = true
-- Keep the chosen headset profile across reconnects.
bluetooth_policy.policy["use-persistent-storage"] = true

-- Applications that should trigger the mic-profile switch even if they do not
-- set media.role = "Communication". Chat Computer's voice capture opens PipeWire
-- with application.name = "Chat Computer" (chatcomputer/voice/audio.py), so list it.
bluetooth_policy.policy["media-role.applications"] =
  { "Chat Computer", "chatcomputer", "python3", "faster-whisper" }
