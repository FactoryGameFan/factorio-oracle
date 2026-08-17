//! Locating prototypes in a `data.raw` dump, and cutting them down to size.

use crate::trim::spec::TrimSpec;
use serde_json::{Map, Value};

/// Finds a prototype by name, searching every prototype type.
///
/// `data.raw` is keyed by prototype TYPE, not by name, and the names a consumer
/// cares about are scattered across types nobody would guess: a pumpjack is a
/// `mining-drill`, a stone wall is a `wall`. Searching every type is cheaper
/// than maintaining a name to type table that silently rots when Factorio
/// reclassifies something.
///
/// The catch is that most names exist more than once. Measured on 2.1.14, all
/// ten of FactorioTools' names appear in three types and `stone-wall` appears
/// in four. `data.raw["item"]["pumpjack"]` is a real prototype; it is simply
/// the item you carry, and it has none of the geometry. Preferring the
/// candidate that has a `collision_box` picks the placeable entity with no
/// hardcoded table.
///
/// When nothing has one there is no right answer, only a stable one, so the
/// first in sorted order wins. `serde_json::Map` is a `BTreeMap` here, so
/// iteration is already sorted.
pub fn find_prototype<'a>(raw: &'a Map<String, Value>, name: &str) -> Option<(String, &'a Value)> {
    let candidates: Vec<(String, &Value)> = raw
        .iter()
        .filter_map(|(kind, protos)| {
            protos
                .as_object()
                .and_then(|o| o.get(name))
                .map(|p| (kind.clone(), p))
        })
        .collect();

    candidates
        .iter()
        .find(|(_, p)| p.get("collision_box").is_some())
        .cloned()
        .or_else(|| candidates.into_iter().next())
}

/// Keeps only the connection keys the caller asked for.
fn trim_connections(fluid_box: &Value, spec: &TrimSpec) -> Vec<Value> {
    let Some(connections) = fluid_box.get("pipe_connections").and_then(|v| v.as_array()) else {
        return vec![];
    };
    connections
        .iter()
        .map(|connection| {
            let mut kept = Map::new();
            for key in &spec.connection_fields {
                if let Some(value) = connection.get(key) {
                    kept.insert(key.clone(), value.clone());
                }
            }
            Value::Object(kept)
        })
        .collect()
}

/// Cuts one prototype down to the fields the caller asked for.
///
/// A field the prototype does not have is skipped rather than written as null.
/// One flat allowlist covers every prototype type, so most fields are absent
/// from most prototypes, and a null would be a claim the game never made.
///
/// `prototypeType` is recorded because it is the thing a name lookup had to
/// discover, and because a reclassification is worth seeing in the diff.
pub fn trim_entity(kind: &str, proto: &Value, spec: &TrimSpec) -> Value {
    let mut trimmed = Map::new();
    trimmed.insert("prototypeType".to_string(), Value::String(kind.to_string()));

    for field in &spec.entity_fields {
        if let Some(value) = proto.get(field) {
            trimmed.insert(field.clone(), value.clone());
        }
    }

    for box_name in &spec.fluid_boxes {
        let Some(fluid_box) = proto.get(box_name) else {
            continue;
        };
        let connections = trim_connections(fluid_box, spec);
        // An empty list says nothing and would churn the diff whenever a box
        // gains or loses a connection, so the box is left out entirely.
        if connections.is_empty() {
            continue;
        }
        let mut kept = Map::new();
        kept.insert("pipe_connections".to_string(), Value::Array(connections));
        trimmed.insert(box_name.clone(), Value::Object(kept));
    }

    Value::Object(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dump() -> Map<String, Value> {
        // The shape that matters: the same name in several types, only one of
        // which is the placeable entity. Measured on 2.1.14, every one of
        // FactorioTools' ten names appears in three or four types.
        json!({
            "item": {
                "pumpjack": { "stack_size": 20 },
                "stone-wall": { "stack_size": 100 }
            },
            "recipe": {
                "pumpjack": { "ingredients": [] },
                "stone-wall": { "ingredients": [] }
            },
            "mining-drill": {
                "pumpjack": { "collision_box": [[-1.2, -1.2], [1.2, 1.2]] }
            },
            "wall": {
                "stone-wall": { "collision_box": [[-0.29, -0.29], [0.29, 0.29]] }
            },
            "technology": {
                "stone-wall": { "unit": {} }
            },
            "not-an-object": 42
        })
        .as_object()
        .unwrap()
        .clone()
    }

    #[test]
    fn prefers_the_candidate_that_has_a_collision_box() {
        // data.raw["item"]["pumpjack"] is a real prototype. It is just the wrong
        // one, and it has none of the geometry. Preferring the candidate with a
        // collision_box picks the entity without a name to type table that
        // silently rots when Factorio reclassifies something.
        let raw = dump();
        let (kind, proto) = find_prototype(&raw, "pumpjack").unwrap();
        assert_eq!(kind, "mining-drill");
        assert!(proto.get("collision_box").is_some());
    }

    #[test]
    fn picks_the_entity_even_when_four_types_share_the_name() {
        let raw = dump();
        let (kind, _) = find_prototype(&raw, "stone-wall").unwrap();
        assert_eq!(kind, "wall");
    }

    #[test]
    fn returns_none_for_a_name_no_type_has() {
        let raw = dump();
        assert!(find_prototype(&raw, "quantum-pumpjack").is_none());
    }

    #[test]
    fn a_type_that_is_not_an_object_is_skipped_rather_than_panicking() {
        // data.raw is not uniformly a map of maps.
        let raw = dump();
        assert!(find_prototype(&raw, "not-an-object").is_none());
    }

    fn spec_for_tests() -> TrimSpec {
        serde_json::from_value(json!({
            "entities": ["pumpjack"],
            "entity_fields": ["collision_box", "module_slots", "energy_usage"],
            "connection_fields": ["position", "positions", "flow_direction",
                                  "max_underground_distance"],
            "fluid_boxes": ["fluid_box", "output_fluid_box", "input_fluid_box"]
        }))
        .unwrap()
    }

    #[test]
    fn keeps_the_asked_for_fields_and_records_the_type() {
        let proto = json!({
            "collision_box": [[-1.2, -1.2], [1.2, 1.2]],
            "module_slots": 2,
            "unwanted_graphics": { "layers": [1, 2, 3] }
        });
        let trimmed = trim_entity("mining-drill", &proto, &spec_for_tests());
        assert_eq!(trimmed["prototypeType"], "mining-drill");
        assert_eq!(trimmed["module_slots"], 2);
        assert!(trimmed.get("unwanted_graphics").is_none());
    }

    #[test]
    fn a_field_the_prototype_does_not_have_is_skipped_not_nulled() {
        // One flat allowlist covers every type, so most fields are absent on
        // most prototypes. A null would be a claim the game never made.
        let proto = json!({ "collision_box": [[0, 0], [1, 1]] });
        let trimmed = trim_entity("pipe", &proto, &spec_for_tests());
        assert!(trimmed.get("module_slots").is_none());
        assert!(!trimmed.as_object().unwrap().contains_key("energy_usage"));
    }

    #[test]
    fn keeps_only_the_asked_for_keys_inside_a_pipe_connection() {
        // pipe_covers alone is several hundred lines of sprite definitions per
        // entity, and none of it is a fact about geometry.
        let proto = json!({
            "output_fluid_box": {
                "pipe_connections": [
                    {
                        "positions": [[1, -1], [1, 1], [-1, 1], [-1, -1]],
                        "flow_direction": "output",
                        "pipe_covers": { "sheets": "lots of sprites" }
                    }
                ],
                "volume": 1000
            }
        });
        let trimmed = trim_entity("mining-drill", &proto, &spec_for_tests());
        let conn = &trimmed["output_fluid_box"]["pipe_connections"][0];
        assert_eq!(conn["flow_direction"], "output");
        assert_eq!(conn["positions"].as_array().unwrap().len(), 4);
        assert!(conn.get("pipe_covers").is_none());
        // Only pipe_connections survives from the box itself.
        assert!(trimmed["output_fluid_box"].get("volume").is_none());
    }

    #[test]
    fn a_fluid_box_with_no_connections_is_left_out_entirely() {
        // An empty pipe_connections list says nothing, and emitting it would
        // churn the diff whenever a box gains or loses one.
        let proto = json!({ "fluid_box": { "volume": 100 } });
        let trimmed = trim_entity("pipe", &proto, &spec_for_tests());
        assert!(trimmed.get("fluid_box").is_none());
    }

    #[test]
    fn the_four_position_output_box_two_point_one_introduced_survives() {
        // Factorio 2.1 changed the pumpjack's output fluid box from 2 distinct
        // corners to 4, one per rotation. That is the exact kind of change this
        // fixture exists to make visible, so it must come through intact.
        let proto = json!({
            "output_fluid_box": {
                "pipe_connections": [{
                    "direction": 0,
                    "positions": [[1, -1], [1, 1], [-1, 1], [-1, -1]],
                    "flow_direction": "output"
                }]
            }
        });
        let mut spec = spec_for_tests();
        spec.connection_fields.push("direction".to_string());
        let trimmed = trim_entity("mining-drill", &proto, &spec);
        let conn = &trimmed["output_fluid_box"]["pipe_connections"][0];
        assert_eq!(
            conn["positions"],
            json!([[1, -1], [1, 1], [-1, 1], [-1, -1]])
        );
        assert_eq!(conn["direction"], 0);
    }

    #[test]
    fn the_fallback_is_alphabetical_so_it_is_deterministic() {
        // When nothing has a collision_box there is no right answer, only a
        // stable one. The Python script took whichever type came first in the
        // document; sorted order is the same idea without depending on how the
        // game happened to serialise the file.
        let raw = json!({
            "zebra": { "ghost": { "a": 1 } },
            "alpha": { "ghost": { "b": 2 } }
        })
        .as_object()
        .unwrap()
        .clone();
        let (kind, _) = find_prototype(&raw, "ghost").unwrap();
        assert_eq!(kind, "alpha");
    }
}
