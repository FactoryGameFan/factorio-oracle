//! Locating prototypes in a `data.raw` dump, and cutting them down to size.

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
