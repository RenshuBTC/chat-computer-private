-- ClaudeOS WirePlumber policy: auto-switch Bluetooth to the headset (mic)
-- profile whenever an input stream opens, and back to A2DP when it closes.
--
-- This overlays bluetooth_policy from the stock policy.lua.d/10-default-policy.
-- policy-bluetooth.lua switches to a profile with an input route when it sees a
-- stream whose media.role is "Communication" OR whose application.name is in
-- media-role.applications. ClaudeOS's own capture node is added below so the
-- AirPods flip to HFP automatically when ClaudeOS starts listening.

bluetooth_policy = bluetooth_policy or {}
bluetooth_policy.policy = bluetooth_policy.policy or {}

-- Switch to the headset profile in the presence of an input stream.
bluetooth_policy.policy["media-role.use-headset-profile"] = true
-- Keep the chosen headset profile across reconnects.
bluetooth_policy.policy["use-persistent-storage"] = true

-- Applications that should trigger the mic-profile switch even if they do not
-- set media.role = "Communication". ClaudeOS's voice capture opens PipeWire
-- with application.name = "ClaudeOS" (see runtime claudeos.voice), so list it.
bluetooth_policy.policy["media-role.applications"] =
  { "ClaudeOS", "claudeos", "python3", "faster-whisper" }
