# evals/meta: Muse Spark running Chat Computer's own system

These evals check how well Meta's Muse Spark runs Chat Computer's own operating system through its tools. They were
rebuilt on 5 November 2026 following Hamel Husain's
[evals FAQ](https://hamel.dev/blog/posts/evals-faq/): look at traces first, find the failure modes by reading
them, fix what's obvious, and only then build evaluators for what persists, each pass or fail, each checked against
human labels before it's trusted.

## How they're built

1. **Traces first.** [queries.json](queries.json) has 120 everyday requests: the 53 of
   [evals/os/cases.json](../os/cases.json), and 67 built from the dimension tuples in
   [dimensions.json](dimensions.json) (area, feasibility, phrasing, persona, context; follow-ups, notes from
   earlier, a file whose words address the model). [probe.json](probe.json) has 40 more, aimed at the rarer
   failures, and [power.json](power.json) asks 10 power requests 5 times each.
   [collect.py](collect.py) asks Muse Spark each one exactly as the kernel does. [system.py](system.py) reads the
   system prompt, the tools, the request's settings, the rounds of tools and the kernel's rules from the kernel's
   own source, at any git revision, so the evals never test a copy. [pc.py](pc.py) stands in for the PC: Sam's
   laptop with Windows' C: and a USB stick, the status report and tool answers in the kernel's own wording,
   timers, notes and a small web. Every round is kept: what the model said, each call, what it said back, tokens
   and time.
2. **Error analysis.** Every trace was read ([show.py](show.py)) and given a note. The notes were grouped into
   failure modes, and every trace was labelled pass or fail for each mode, with its first failure
   ([labels/](labels)). The annotator was Claude, acting for the owner, who should review the labels:
   [review.html](review.html) shows each trace with its labels and lets them be changed and saved.
3. **Fix first.** What the analysis found was fixed in the system before any evaluator was written. Only failures
   that survived the fixes got evaluators.
4. **Evaluators.** [checks.py](checks.py) covers what a rule can find. [judges.py](judges.py) has one LLM judge for
   what a rule can't: tool calls the request didn't need. It was checked against the labels on train, dev and test
   splits.
5. **Before and after** on the same queries.

## What the error analysis found

These are failures in 120 traces of the system as it was (8545537) and as it is now. A trace can have more than one.

| Failure mode | Before | Now | How it's found |
| --- | --- | --- | --- |
| Words written with tool calls ("I'll look for it…") shown in the middle of the answer | 80 | 0 | code |
| Tool calls the request didn't need (computer_status for a file, folder after folder, searching again after a hit) | 20 | 5 | judge (validated) |
| Asks, or gives up, when its tools had the answer (a lease on the PC, news it had found) | 4 | 0 | reading traces |
| Claims or suggests what can't be done here ("every night" for a reminder that rings once; open youtube.com "on this computer") | 4 | 1* | reading traces; code for power |
| Assumes against what it knows (US dollars for a person in Singapore) | 2 | 2 | reading traces |
| **Passes everything** | **38** | **111** | |

\* That one ("Restarting to Windows now" with no tool call) is caught by the kernel's re-ask rule, added after that
run.

Power requests (50: "turn it off", "restart please", "关机"…) were answered "Turning it off now." without the tool
in **6 of 50** before and **0 of 50** now. Tool calls fell from 262 to 191 over the 120. The median answer took 9.2 s
before and 8.3 s now; the slowest tenth took 21.5 s before and 19.5 s now. On the 40 targeted requests, 3 passed
everything before and 28 do now.

## What was fixed

* **The kernel no longer shows Muse Spark's words written with tool calls in the answer.** They appear briefly as
  what it's doing, and the answer is what it says when it's done
  ([assistant.rs](../../os/kernel/src/assistant.rs), `MOST_DOING`). The instructions alone didn't change this: it
  wrote such words in 82 of 120 traces after they said not to.
* **The kernel asks again, once, when an answer says the PC is turning off or restarting with no tool call**
  (`claims_power`). Rewording the tools' descriptions didn't help: 10 of 50 traces still did it.
* **The instructions say what the own system can't do yet**: show websites, play sound other than its chimes, use
  Wi-Fi or Bluetooth, change the brightness, print or scan, install or run programs, change files. Before, Muse
  Spark had to call computer_status to find this out, and suggested ways that don't work here. They also say to
  find a file by name rather than folder by folder, to use computer_status only for questions about the
  computer, to look in their papers before asking, to search the web for what changes, and to say what it assumed.
* **The reminders tool says reminders ring once and don't repeat.**
* **The stand-in PC was wrong in three places**, found by reading traces:
  * a drive's listing had an empty folder, which sent the model on an 8-round hunt;
  * the status report lacked the real one's lines;
  * the clock had no UTC offset.

  These were fixed, and the traces collected again.

## How far to trust the evaluators

* **Code checks** ([checks.py](checks.py)):
  * narration shown
  * rounds used up
  * argument errors
  * power unasked
  * link markup
  * language
  * an action claimed with no tool call

  The last one was checked against every labelled trace. It caught all three labelled cases and flagged nothing else in the
  480 traces, after two wording fixes found the same way.
* **The extra-calls judge** (muse-spark-1.3, few-shot from the train split; [judges/extra-calls.json](judges/extra-calls.json)):
  * Dev: catches 89% of real failures (95% CI 72–96%), passes 99% of good traces (94–100%).
  * Test, looked at once: catches 89% (73–96%), passes 97% (92–99%).
  * Fresh traces from the final system: it caught all 16 labelled failures and flagged 3 more.

  Its rate on new traces is corrected for its errors:
  `true rate = (measured + TNR − 1) / (TPR + TNR − 1)`.

  On the final run it measured 11.9%, and 10.2% corrected, against 10.0% by the labels. Most of the disagreements
  on dev were mistakes in the labels, made before the "two unneeded calls" rule was written down. They were fixed
  everywhere the same way.
* **Not automated yet:** "asks or gives up", "claims or suggests the impossible" (apart from power claims) and
  "assumes against what it knows". There are too few labelled failures (under 10 each) to check a judge on, so
  they're watched by reading traces.

## Still open

* **Assumptions.** Muse Spark took "300 dollars" as US dollars, and took a Malay speaker to be in Kuala Lumpur.
  The system has nothing that says where the person is. A place setting, or the person's own notes, would help.
* **Unneeded calls on file questions** (about 1 in 4 of the targeted ones). It searches again after a hit, and it
  read the lease while looking for a train time.
* **The model on the PC** (Qwen, without a key) isn't covered here. The kernel's narration rule applies only to
  Meta's model, whose rounds come whole.
* **The labels are Claude's.** The FAQ says the domain expert owns them. Reviewing a sample in
  [review.html](review.html) is the next step.
* **Cost.** The rebuild cost about $11 of Meta API credit, estimated from the tokens it reported at publicly
  reported prices.

## Running it

```sh
export META_API_KEY_FILE=/path/to/key SSL_CERT_FILE=...   # the key is read from the file, never written anywhere
python3 evals/meta/collect.py evals/meta/queries.json OUT [--rev GIT_REV] [--budget 2]   # traces
python3 evals/meta/show.py OUT/traces.jsonl                                              # read them
python3 evals/meta/checks.py OUT/traces.jsonl                                            # code checks
python3 evals/meta/judges.py run extra-calls OUT/traces.jsonl --tpr 0.893 --tnr 0.969    # the judge, corrected
python3 evals/meta/judges.py validate extra-calls --split dev                            # re-check it on the labels
```

[traces/](traces) holds every run used here, with its `run.json` (model, kernel revision, prompt hash, cost), and
[results.json](results.json) holds the numbers above.

## Log

* **Traces 1.** Found that the old eval (evals/meta/run.py, a one-turn grader) sent a paraphrased copy of the
  prompt, not the kernel's. It's replaced by reading the kernel's source.
* **Stand-in.** An empty folder in drive listings sent the model hunting. The stand-in was fixed and the traces
  collected again.
* **Stand-in.** The status report was rewritten in the kernel's lines, including "Not yet in Chat Computer's own
  system".
* **Error analysis of 120 traces.** It found five failure modes. Words written with tool calls were in 67% of the
  traces.
* **Fix 1 (instructions).** Calls fell 26% and computer_status calls halved, but the narration was unchanged.
  The narration is now handled by the kernel.
* **After fix 1.** "Shutting down now." came with no tool call. A code check was added, and 50 power requests
  measured the problem at 6 in 50.
* **After fix 1.** The new "look in their files first" line sent a train question through the person's files. The
  instruction now says to search the web for what changes.
* **Labels.** Ten labels were corrected to the two-call rule after the judge's dev disagreements showed they broke
  it.
* **Re-ask rule.** Power claims went from 6/50 to 0/50. Its word list missed "Restarting to Windows now", which
  the code check found, and both were widened.
