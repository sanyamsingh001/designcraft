//! Exact native-origin blank-EOF state, carried in a standard Story script label.
//! Ordinary IDML PSR/CSR import remains the fallback; no child-story typing stage.
use designcraft_doc::{CharAttrs, Story};
use serde_json::{Value, json};

use crate::xml::{El, Node};

const KEY: &str = "ai.storyteller.designcraft.eofTyping.v1";
const LIMIT: usize = 64 * 1024;

// Attribute order and XML indentation are immaterial. Bound work before cloning input.
fn witness(el: &El, depth: usize, budget: &mut usize) -> Option<Value> {
    if depth > 16 {
        return None;
    }
    *budget = budget.checked_sub(1 + el.name.len())?;
    let mut attrs = std::collections::BTreeMap::new();
    for (key, value) in &el.attrs {
        *budget = budget.checked_sub(key.len().checked_add(value.len())?)?;
        if attrs.insert(key, value).is_some() {
            return None;
        }
    }
    let mut children = Vec::new();
    for node in &el.children {
        match node {
            Node::El(child) => children.push(witness(child, depth + 1, budget)?),
            Node::Text(text) | Node::CData(text) if text.trim().is_empty() => {}
            Node::Text(text) | Node::CData(text) => {
                *budget = budget.checked_sub(text.len())?;
                children.push(json!(["text", text]));
            }
            Node::Pi(target, value) => {
                *budget = budget.checked_sub(target.len().checked_add(value.len())?)?;
                children.push(json!(["pi", target, value]));
            }
        }
    }
    Some(json!([el.name, attrs, children]))
}

pub(crate) fn attach(el: &mut El, story: &Story) {
    if el.local() != "Story" || !story.text.ends_with('\n') {
        return;
    }
    let Some(pf) = story.paras.last().filter(|pf| !pf.chars.is_empty() || pf.eof_inherited_chars.is_some()) else { return };
    let Some(psr) = el.find_all("ParagraphStyleRange").last() else { return };
    let mut budget = LIMIT;
    let Some(witness) = witness(psr, 0, &mut budget) else { return };
    let payload = json!({"length": story.len(), "paragraphs": story.paras.len(), "witness": witness,
        "chars": pf.chars, "inherited": pf.eof_inherited_chars})
    .to_string();
    if payload.len() > LIMIT {
        return;
    }
    // story_el currently has no Properties child. Keep that existing local invariant.
    el.children
        .insert(0, Node::El(El::new("Properties").child(El::new("Label").child(El::new("KeyValuePair").attr("Key", KEY).attr("Value", payload)))));
}

// The sparse CharAttrs decoder distinguishes explicit null values from omission.
fn attrs(value: &Value) -> Option<CharAttrs> {
    value.as_object()?;
    serde_json::from_value(value.clone()).ok()
}

pub(crate) fn restore(el: &El, story: &mut Story) {
    if el.local() != "Story" || !story.text.ends_with('\n') {
        return;
    }
    let Some(label) = el.prop_el("Label") else { return };
    let mut entries = label.find_all("KeyValuePair").filter(|pair| pair.get("Key") == Some(KEY));
    let Some(payload) = entries.next().and_then(|pair| pair.get("Value")).filter(|value| value.len() <= LIMIT) else { return };
    if entries.next().is_some() {
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(payload) else { return };
    if value.get("length").and_then(Value::as_u64) != Some(story.len() as u64)
        || value.get("paragraphs").and_then(Value::as_u64) != Some(story.paras.len() as u64)
    {
        return;
    }
    let Some(psr) = el.find_all("ParagraphStyleRange").last() else { return };
    let mut budget = LIMIT;
    let Some(current) = witness(psr, 0, &mut budget) else { return };
    if value.get("witness") != Some(&current) {
        return;
    }
    let Some(chars) = value.get("chars").and_then(attrs) else { return };
    let inherited = match value.get("inherited") {
        Some(Value::Null) => None,
        Some(value) => match attrs(value) {
            Some(attrs) => Some(attrs),
            None => return,
        },
        None => return,
    };
    if let Some(pf) = story.paras.last_mut() {
        pf.chars = chars;
        pf.eof_inherited_chars = inherited;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use designcraft_doc::{ParaFormat, StoryId};

    fn fixture() -> (Story, El) {
        let mut story = Story::with_text(StoryId(1), "A\n", ParaFormat::default());
        story.paras[1].chars = CharAttrs { size: Some(30.0), underline_weight: Some(None), ..Default::default() };
        story.paras[1].eof_inherited_chars = Some(CharAttrs { size: Some(24.0), ..Default::default() });
        let el = El::new("Story").child(El::new("ParagraphStyleRange").attr("PointSize", "30").child(El::new("CharacterStyleRange")));
        (story, el)
    }

    #[test]
    fn stage_and_baseline_survive_actual_xml_encoding() {
        let (source, mut el) = fixture();
        attach(&mut el, &source);
        let parsed = crate::xml::parse(&crate::xml::document(&el)).unwrap();
        let mut back = Story::with_text(StoryId(2), "A\n", ParaFormat::default());
        restore(&parsed, &mut back);
        assert_eq!(back.paras[1].chars, source.paras[1].chars);
        assert_eq!(back.paras[1].eof_inherited_chars, source.paras[1].eof_inherited_chars);
        assert_eq!(back.chars, source.chars, "the real LF run is not overwritten");
    }

    #[test]
    fn nullable_layers_keep_explicit_auto_values() {
        let a = attrs(&json!({"leadingAki":null,"trailingAki":null,"underlineWeight":null,
            "underlineOffset":null,"strikethroughWeight":null,"strikethroughOffset":null}))
        .unwrap();
        assert_eq!(a.leading_aki, Some(None));
        assert_eq!(a.trailing_aki, Some(None));
        assert_eq!(a.underline_weight, Some(None));
        assert_eq!(a.underline_offset, Some(None));
        assert_eq!(a.strikethrough_weight, Some(None));
        assert_eq!(a.strikethrough_offset, Some(None));
        assert!(attrs(&json!({})).unwrap().is_empty());
    }

    #[test]
    fn absent_and_explicitly_empty_baselines_remain_distinct() {
        for inherited in [None, Some(CharAttrs::default())] {
            let (mut source, mut el) = fixture();
            source.paras[1].eof_inherited_chars = inherited.clone();
            attach(&mut el, &source);
            let parsed = crate::xml::parse(&crate::xml::document(&el)).unwrap();
            let mut back = Story::with_text(StoryId(2), "A\n", ParaFormat::default());
            restore(&parsed, &mut back);
            assert_eq!(back.paras[1].chars, source.paras[1].chars);
            assert_eq!(back.paras[1].eof_inherited_chars, inherited);
        }
    }

    #[test]
    fn changed_final_psr_and_child_elements_do_not_replay_metadata() {
        let (source, mut el) = fixture();
        attach(&mut el, &source);
        let plain = Story::with_text(StoryId(2), "A\n", ParaFormat::default());
        for kind in ["Cell", "Footnote"] {
            let mut child = el.clone();
            child.name = kind.into();
            let mut back = plain.clone();
            restore(&child, &mut back);
            assert_eq!(back, plain);
        }
        for node in &mut el.children {
            if let Node::El(psr) = node
                && psr.local() == "ParagraphStyleRange"
            {
                psr.set("PointSize", 22);
            }
        }
        let mut back = plain.clone();
        restore(&el, &mut back);
        assert_eq!(back, plain);
    }

    #[test]
    fn empty_nonempty_and_unstaged_controls_do_not_gain_metadata() {
        for text in ["", "A", "A\n"] {
            let story = Story::with_text(StoryId(1), text, ParaFormat::default());
            let mut el = El::new("Story").child(El::new("ParagraphStyleRange"));
            let before = el.clone();
            attach(&mut el, &story);
            assert_eq!(el, before);
        }
    }
}
