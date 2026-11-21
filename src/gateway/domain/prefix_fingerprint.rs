//! Cache-break diagnostics: where did this request's prefix first differ from the
//! closest earlier request of the same session and class?
//!
//! Anthropic's prompt cache matches on the request prefix (tools, system, then
//! messages in order). A cache read shorter than the previous request's context
//! means something early in the prefix changed. The usage log has token counts but
//! not content, so each request is fingerprinted and, when the prefix diverges, a
//! description of what kind of block moved is produced. Only hashes are kept in memory
//! and only a short description of the first differing block reaches the log.
//!
//! Reading a request body into a [`RequestFp`] and writing a [`PrefixDiff`] to the log is
//! the infrastructure adapter's job; this module compares fingerprints.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// Earlier requests remembered per session and class. Parallel subagents interleave,
/// so one chain is not enough; more than this and the match gets slow and noisy.
const KEEP: usize = 12;

/// One content block of a message: its hash and what the log shows of it.
pub struct BlockFp {
    pub hash: u64,
    pub kind: String,
    pub bytes: usize,
    /// The first characters of its text, for text and thinking blocks.
    pub text: Option<String>,
}

pub struct MessageFp {
    pub hash: u64,
    pub role: Option<String>,
    pub blocks: Vec<BlockFp>,
}

/// A request reduced to what the comparison needs.
pub struct RequestFp {
    pub system: u64,
    pub tools: u64,
    pub msgs: Vec<MessageFp>,
}

/// The message at the divergence point: its role and the first block that differs
/// from the earlier request's message at the same position (if there was one).
pub struct Divergence {
    pub role: Option<String>,
    pub blocks: usize,
    pub prev_blocks: Option<usize>,
    pub block_index: Option<usize>,
    pub block: Option<BlockFp>,
}

pub struct PrefixDiff {
    pub intact: bool,
    pub tail_only: bool,
    pub system_changed: bool,
    pub tools_changed: bool,
    pub prev_messages: usize,
    pub shared_messages: usize,
    pub first_diff: Option<Divergence>,
}

/// What is kept of an earlier request.
struct Fp {
    system: u64,
    tools: u64,
    msgs: Vec<u64>,
    /// Per message, the hash of each content block (to find the block that changed).
    blocks: Vec<Vec<u64>>,
}

#[derive(Default)]
pub struct PrefixWatch {
    recent: Mutex<HashMap<String, VecDeque<Fp>>>,
}

fn describe_message(msg: Option<MessageFp>, old_blocks: Option<&Vec<u64>>) -> Option<Divergence> {
    let msg = msg?;
    let at = (0..msg.blocks.len()).find(|&j| old_blocks.and_then(|o| o.get(j)) != Some(&msg.blocks[j].hash));
    Some(Divergence {
        role: msg.role,
        blocks: msg.blocks.len(),
        prev_blocks: old_blocks.map(Vec::len),
        block_index: at,
        block: at.and_then(|j| msg.blocks.into_iter().nth(j)),
    })
}

impl PrefixWatch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Compare `req` with the earlier request of this session and class that shares
    /// the most prefix, remember it, and describe the difference. Returns `None` for
    /// a request with no comparable predecessor.
    pub fn observe(&self, session: &str, class: &str, req: RequestFp) -> Option<PrefixDiff> {
        let fp = Fp {
            system: req.system,
            tools: req.tools,
            msgs: req.msgs.iter().map(|m| m.hash).collect(),
            blocks: req.msgs.iter().map(|m| m.blocks.iter().map(|b| b.hash).collect()).collect(),
        };
        let mut messages: Vec<Option<MessageFp>> = req.msgs.into_iter().map(Some).collect();
        let key = format!("{session}|{class}");
        let mut all = self.recent.lock().unwrap();
        let list = all.entry(key).or_default();

        // Closest predecessor: same system and tools first, then longest shared message prefix.
        let common = |p: &Fp| p.msgs.iter().zip(&fp.msgs).take_while(|(a, b)| a == b).count();
        let best = list.iter().enumerate().max_by_key(|(i, p)| ((p.system == fp.system) as u8 + (p.tools == fp.tools) as u8, common(p), *i));
        let out = best.map(|(_, p)| {
            let n = common(p);
            let (sys, tools) = (p.system != fp.system, p.tools != fp.tools);
            // The prefix is intact when the earlier request is wholly contained in this one.
            let intact = !sys && !tools && n == p.msgs.len();
            let diverged = if !intact && !sys && !tools {
                // `n` is where the shared prefix ends: the earlier message there was edited
                // (or the conversation was rewound or compacted and it no longer exists).
                describe_message(messages.get_mut(n).and_then(Option::take), p.blocks.get(n))
            } else {
                None
            };
            // Only the earlier request's last message changed (Claude Code rewrites the
            // trailing message, e.g. its reminders). On 291 logged main requests this
            // cost ~6k cache tokens each, against 45k-725k for a real break, so count it apart.
            let tail_only = !intact && !sys && !tools && n + 1 == p.msgs.len();
            PrefixDiff {
                intact,
                tail_only,
                system_changed: sys,
                tools_changed: tools,
                prev_messages: p.msgs.len(),
                shared_messages: n,
                first_diff: diverged,
            }
        });

        list.push_back(fp);
        if list.len() > KEEP {
            list.pop_front();
        }
        out
    }
}
