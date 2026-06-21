#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::ids::{MemoryId, TenantId, UserId};
use crate::memory::{MemoryKind, MemoryRecord};

use super::locomo::{is_locomo_session_key, sanitize_id_fragment, split_locomo_dia_refs};
use super::LocomoMemorySource;

// ---------------------------------------------------------------------------
// Memory construction from LOCOMO dataset
// ---------------------------------------------------------------------------

pub(super) fn locomo_memories(
    conv_idx: usize,
    entry: &serde_json::Map<String, Value>,
    tenant: &TenantId,
    user: &UserId,
    source: LocomoMemorySource,
) -> (Vec<MemoryRecord>, BTreeMap<String, Vec<String>>) {
    let mut memories = Vec::new();
    let mut dia_to_memory = BTreeMap::new();

    if source.includes_raw() {
        let (mut raw_memories, raw_map) = locomo_raw_memories(conv_idx, entry, tenant, user);
        memories.append(&mut raw_memories);
        merge_dia_map(&mut dia_to_memory, raw_map);
    }
    if source.includes_observation() {
        let (mut observation_memories, observation_map) =
            locomo_observation_memories(conv_idx, entry, tenant, user);
        memories.append(&mut observation_memories);
        merge_dia_map(&mut dia_to_memory, observation_map);
    }
    if source.includes_fact_summary() {
        let (mut summary_memories, summary_map) =
            locomo_speaker_summary_memories(conv_idx, entry, tenant, user);
        memories.append(&mut summary_memories);
        merge_dia_map(&mut dia_to_memory, summary_map);
    }

    (memories, dia_to_memory)
}

fn locomo_raw_memories(
    conv_idx: usize,
    entry: &serde_json::Map<String, Value>,
    tenant: &TenantId,
    user: &UserId,
) -> (Vec<MemoryRecord>, BTreeMap<String, Vec<String>>) {
    let mut memories = Vec::new();
    let mut dia_to_memory = BTreeMap::new();
    let Some(conversation) = entry.get("conversation").and_then(Value::as_object) else {
        return (memories, dia_to_memory);
    };
    for (key, value) in conversation {
        if !is_locomo_session_key(key, value) {
            continue;
        }
        let session_date = conversation
            .get(&format!("{key}_date_time"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let Some(turns) = value.as_array() else {
            continue;
        };
        let mut episode_lines = Vec::new();
        let mut episode_dia_ids = Vec::new();
        let mut episode_entities = BTreeSet::new();
        for (turn_index, turn) in turns.iter().enumerate() {
            let dia_id = turn.get("dia_id").and_then(Value::as_str).unwrap_or("");
            let text = turn.get("text").and_then(Value::as_str).unwrap_or("");
            if dia_id.is_empty() || text.trim().is_empty() {
                continue;
            }
            let speaker = turn.get("speaker").and_then(Value::as_str).unwrap_or("");
            if let Some(line) = locomo_turn_context_line(turn) {
                episode_lines.push(format!("- {line}"));
            }
            episode_dia_ids.push(dia_id.to_string());
            if !speaker.is_empty() {
                episode_entities.insert(speaker.to_string());
            }
            let memory_id = MemoryId::new(format!(
                "mem_locomo_{}_{}",
                conv_idx,
                sanitize_id_fragment(dia_id)
            ));
            let current = if session_date.is_empty() {
                format!("[{dia_id}] {speaker}: {text}")
            } else {
                format!("[{dia_id}, said on {session_date}] {speaker}: {text}")
            };
            let mut context = Vec::new();
            if let Some(previous) = turn_index
                .checked_sub(1)
                .and_then(|index| turns.get(index))
                .and_then(locomo_turn_context_line)
            {
                context.push(format!("Previous turn: {previous}"));
            }
            if let Some(next) = turns.get(turn_index + 1).and_then(locomo_turn_context_line) {
                context.push(format!("Next turn: {next}"));
            }
            let body = if context.is_empty() {
                current
            } else {
                format!("{current}\n{}", context.join("\n"))
            };
            let mut memory = MemoryRecord::new_user_preference(
                memory_id.clone(),
                tenant.clone(),
                user.clone(),
                body,
            );
            memory.kind = MemoryKind::Fact;
            memory.tags = vec!["locomo".to_string(), key.clone()];
            if !speaker.is_empty() {
                memory.entities = vec![speaker.to_string()];
            }
            add_dia_memory(&mut dia_to_memory, dia_id, memory_id.to_string());
            if let Some(previous_dia_id) = turn_index
                .checked_sub(1)
                .and_then(|index| turns.get(index))
                .and_then(locomo_turn_dia_id)
            {
                add_dia_memory(&mut dia_to_memory, previous_dia_id, memory_id.to_string());
            }
            if let Some(next_dia_id) = turns.get(turn_index + 1).and_then(locomo_turn_dia_id) {
                add_dia_memory(&mut dia_to_memory, next_dia_id, memory_id.to_string());
            }
            memories.push(memory);
        }
        if !episode_lines.is_empty() {
            let memory_id = MemoryId::new(format!(
                "mem_locomo_{}_{}_episode",
                conv_idx,
                sanitize_id_fragment(key)
            ));
            let header = if session_date.is_empty() {
                format!("[{key} episode]")
            } else {
                format!("[{key} episode, said on {session_date}]")
            };
            let mut memory = MemoryRecord::new_user_preference(
                memory_id.clone(),
                tenant.clone(),
                user.clone(),
                format!("{header}\n{}", episode_lines.join("\n")),
            );
            memory.kind = MemoryKind::Fact;
            memory.importance = 0.7;
            memory.tags = vec!["locomo".to_string(), key.clone(), "episode".to_string()];
            memory.entities = episode_entities.into_iter().collect();
            for dia_id in episode_dia_ids {
                add_dia_memory(&mut dia_to_memory, &dia_id, memory_id.to_string());
            }
            memories.push(memory);
        }
    }
    (memories, dia_to_memory)
}

fn locomo_turn_dia_id(turn: &Value) -> Option<&str> {
    let dia_id = turn.get("dia_id").and_then(Value::as_str)?.trim();
    (!dia_id.is_empty()).then_some(dia_id)
}

fn locomo_turn_context_line(turn: &Value) -> Option<String> {
    let dia_id = turn.get("dia_id").and_then(Value::as_str)?;
    let text = turn.get("text").and_then(Value::as_str)?.trim();
    if dia_id.is_empty() || text.is_empty() {
        return None;
    }
    let speaker = turn.get("speaker").and_then(Value::as_str).unwrap_or("");
    Some(format!("[{dia_id}] {speaker}: {text}"))
}

fn locomo_observation_memories(
    conv_idx: usize,
    entry: &serde_json::Map<String, Value>,
    tenant: &TenantId,
    user: &UserId,
) -> (Vec<MemoryRecord>, BTreeMap<String, Vec<String>>) {
    let mut memories = Vec::new();
    let mut dia_to_memory = BTreeMap::new();
    let Some(observation) = entry.get("observation").and_then(Value::as_object) else {
        return (memories, dia_to_memory);
    };

    let mut observation_index = 0;
    for (observation_key, speakers_value) in observation {
        let Some(speakers) = speakers_value.as_object() else {
            continue;
        };
        let session = observation_key
            .strip_suffix("_observation")
            .unwrap_or(observation_key);
        for (speaker, facts_value) in speakers {
            let Some(facts) = facts_value.as_array() else {
                continue;
            };
            for fact_value in facts {
                let Some(fact_pair) = fact_value.as_array() else {
                    continue;
                };
                let fact = fact_pair
                    .first()
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                let dia_id = fact_pair
                    .get(1)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                if fact.is_empty() || dia_id.is_empty() {
                    continue;
                }

                let memory_id = MemoryId::new(format!(
                    "mem_locomo_{}_{}_obs_{}",
                    conv_idx,
                    sanitize_id_fragment(dia_id),
                    observation_index
                ));
                observation_index += 1;
                let body = if speaker.is_empty() {
                    format!("[{dia_id} observation] {fact}")
                } else {
                    format!("[{dia_id} observation] {speaker}: {fact}")
                };
                let mut memory = MemoryRecord::new_user_preference(
                    memory_id.clone(),
                    tenant.clone(),
                    user.clone(),
                    body,
                );
                memory.kind = MemoryKind::Fact;
                memory.importance = 0.8;
                memory.tags = vec![
                    "locomo".to_string(),
                    "observation".to_string(),
                    session.to_string(),
                ];
                if !speaker.is_empty() {
                    memory.entities = vec![speaker.to_string()];
                }
                add_dia_memory(&mut dia_to_memory, dia_id, memory_id.to_string());
                memories.push(memory);
            }
        }
    }

    (memories, dia_to_memory)
}

fn locomo_speaker_summary_memories(
    conv_idx: usize,
    entry: &serde_json::Map<String, Value>,
    tenant: &TenantId,
    user: &UserId,
) -> (Vec<MemoryRecord>, BTreeMap<String, Vec<String>>) {
    let mut by_speaker: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let Some(observation) = entry.get("observation").and_then(Value::as_object) else {
        return (Vec::new(), BTreeMap::new());
    };

    for speakers_value in observation.values() {
        let Some(speakers) = speakers_value.as_object() else {
            continue;
        };
        for (speaker, facts_value) in speakers {
            let Some(facts) = facts_value.as_array() else {
                continue;
            };
            for fact_value in facts {
                let Some(fact_pair) = fact_value.as_array() else {
                    continue;
                };
                let fact = fact_pair
                    .first()
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                let dia_id = fact_pair
                    .get(1)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                if fact.is_empty() || dia_id.is_empty() {
                    continue;
                }
                by_speaker
                    .entry(speaker.clone())
                    .or_default()
                    .push((fact.to_string(), dia_id.to_string()));
            }
        }
    }

    let mut memories = Vec::new();
    let mut dia_to_memory = BTreeMap::new();
    for (speaker, facts) in by_speaker {
        if facts.is_empty() {
            continue;
        }
        let speaker_id = if speaker.is_empty() {
            "unknown".to_string()
        } else {
            sanitize_id_fragment(&speaker)
        };
        let memory_id = MemoryId::new(format!("mem_locomo_{}_fact_layer_{}", conv_idx, speaker_id));
        let joined_facts = facts
            .iter()
            .map(|(fact, dia_id)| format!("[{dia_id}] {fact}"))
            .collect::<Vec<_>>()
            .join("; ");
        let body = if speaker.is_empty() {
            format!("[speaker fact-layer summary] {joined_facts}")
        } else {
            format!("[speaker fact-layer summary] {speaker}: {joined_facts}")
        };
        let mut memory = MemoryRecord::new_user_preference(
            memory_id.clone(),
            tenant.clone(),
            user.clone(),
            body,
        );
        memory.kind = MemoryKind::Fact;
        memory.importance = 0.9;
        memory.tags = vec![
            "locomo".to_string(),
            "observation".to_string(),
            "fact-layer".to_string(),
            "summary".to_string(),
        ];
        if !speaker.is_empty() {
            memory.entities = vec![speaker.clone()];
        }
        for (_, dia_id) in facts {
            add_dia_memory(&mut dia_to_memory, &dia_id, memory_id.to_string());
        }
        memories.push(memory);
    }

    (memories, dia_to_memory)
}

// ---------------------------------------------------------------------------
// Dia map helpers
// ---------------------------------------------------------------------------

pub(super) fn merge_dia_map(
    target: &mut BTreeMap<String, Vec<String>>,
    source: BTreeMap<String, Vec<String>>,
) {
    for (dia_id, ids) in source {
        target.entry(dia_id).or_default().extend(ids);
    }
}

fn add_dia_memory(map: &mut BTreeMap<String, Vec<String>>, dia_id: &str, memory_id: String) {
    for dia_id in split_locomo_dia_refs(dia_id) {
        map.entry(dia_id).or_default().push(memory_id.clone());
    }
}
