use crate::annis::db::graphstorage::union::UnionEdgeContainer;
use crate::annis::db::graphstorage::EdgeContainer;
use crate::annis::db::{Graph, ANNIS_NS, TOK};
use crate::annis::errors::*;
use crate::annis::types::{AnnoKey, Annotation, Component, ComponentType, Edge, NodeID};
use crate::update::{GraphUpdate, UpdateEvent};
use csv;
use multimap::MultiMap;
use std;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustc_hash::FxHashMap;

#[derive(Eq, PartialEq, PartialOrd, Ord, Hash, Clone, Debug)]
struct TextProperty {
    segmentation: String,
    corpus_id: u32,
    text_id: u32,
    val: u32,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct TextKey {
    id: u32,
    corpus_ref: Option<u32>,
}

struct Text {
    name: String,
}

struct ParsedCorpusTable {
    toplevel_corpus_name: String,
    corpus_by_preorder: BTreeMap<u32, u32>,
    corpus_id_to_name: BTreeMap<u32, String>,
}

struct TextPosTable {
    token_by_left_textpos: BTreeMap<TextProperty, NodeID>,
    token_by_right_textpos: BTreeMap<TextProperty, NodeID>,
    // maps a token index to an node ID
    token_by_index: BTreeMap<TextProperty, NodeID>,
    // maps a token node id to the token index
    token_to_index: BTreeMap<NodeID, TextProperty>,
    // map as node to it's "left" value
    node_to_left: BTreeMap<NodeID, TextProperty>,
    // map as node to it's "right" value
    node_to_right: BTreeMap<NodeID, TextProperty>,
}

/// Load a c corpus in the legacy relANNIS format from the specified `path`.
///
/// Returns a tuple consisting of the corpus name and the extracted annotation graph.
pub fn load<F>(path: &Path, progress_callback: F) -> Result<(String, Graph)>
where
    F: Fn(&str) -> (),
{
    // convert to path
    let path = PathBuf::from(path);
    if path.is_dir() && path.exists() {
        // check if this is the ANNIS 3.3 import format
        let annis_version_path = path.clone().join("annis.version");
        let is_annis_33 = if annis_version_path.exists() {
            let mut file = File::open(&annis_version_path)?;
            let mut version_str = String::new();
            file.read_to_string(&mut version_str)?;

            version_str == "3.3"
        } else {
            false
        };

        let mut db = Graph::new();
        let (toplevel_corpus_name, id_to_node_name, textpos_table) = {
            let mut update = GraphUpdate::new();

            let (toplevel_corpus_name, id_to_node_name, textpos_table) =
                load_node_and_corpus_tables(&path, &mut update, is_annis_33, &progress_callback)?;

            progress_callback(&format!(
                "committing {} annotation node and corpus structure updates",
                update.len()
            ));
            db.apply_update(&mut update)?;

            (toplevel_corpus_name, id_to_node_name, textpos_table)
        };

        for order_component in db.get_all_components(Some(ComponentType::Ordering), None) {
            db.calculate_component_statistics(&order_component)?;
            db.optimize_impl(&order_component);
        }

        {
            let mut update = GraphUpdate::new();

            load_edge_tables(
                &path,
                &mut update,
                is_annis_33,
                &id_to_node_name,
                &progress_callback,
            )?;

            progress_callback(&format!("committing {} edge updates", update.len()));
            db.apply_update(&mut update)?;
        };

        {
            let mut update = GraphUpdate::new();

            calculate_automatic_coverage_edges(
                &mut update,
                &db,
                &textpos_table,
                &id_to_node_name,
                &progress_callback,
            )?;

            progress_callback(&format!(
                "committing {} automatic generated coverage edge updates",
                update.len()
            ));
            db.apply_update(&mut update)?;
        }

        progress_callback("calculating node statistics");
        Arc::make_mut(&mut db.node_annos).calculate_statistics();

        for c in db.get_all_components(None, None) {
            progress_callback(&format!("calculating statistics for component {}", c));
            db.calculate_component_statistics(&c)?;
            db.optimize_impl(&c);
        }

        progress_callback(&format!(
            "finished loading relANNIS from {}",
            path.to_string_lossy()
        ));

        return Ok((toplevel_corpus_name, db));
    }

    Err(format!("Directory {} not found", path.to_string_lossy()).into())
}

fn load_node_and_corpus_tables<F>(
    path: &PathBuf,
    update: &mut GraphUpdate,
    is_annis_33: bool,
    progress_callback: &F,
) -> Result<(String, FxHashMap<NodeID, String>, TextPosTable)>
where
    F: Fn(&str) -> (),
{
    let corpus_table = parse_corpus_tab(&path, is_annis_33, &progress_callback)?;
    let texts = parse_text_tab(&path, is_annis_33, &progress_callback)?;
    let corpus_id_to_annos = load_corpus_annotation(&path, is_annis_33, &progress_callback)?;

    let (nodes_by_text, id_to_node_name, textpos_table) = load_nodes(
        path,
        update,
        &corpus_table.corpus_id_to_name,
        &corpus_table.toplevel_corpus_name,
        is_annis_33,
        progress_callback,
    )?;

    add_subcorpora(
        update,
        &corpus_table,
        &nodes_by_text,
        &texts,
        &corpus_id_to_annos,
        &id_to_node_name,
        is_annis_33,
    )?;

    Ok((
        corpus_table.toplevel_corpus_name,
        id_to_node_name,
        textpos_table,
    ))
}

fn load_edge_tables<F>(
    path: &PathBuf,
    update: &mut GraphUpdate,
    is_annis_33: bool,
    id_to_node_name: &FxHashMap<NodeID, String>,
    progress_callback: &F,
) -> Result<()>
where
    F: Fn(&str) -> (),
{
    let (pre_to_component, pre_to_edge) = {
        let component_by_id = load_component_tab(path, is_annis_33, progress_callback)?;

        let (pre_to_component, pre_to_edge) = load_rank_tab(
            path,
            update,
            &component_by_id,
            id_to_node_name,
            is_annis_33,
            progress_callback,
        )?;

        (pre_to_component, pre_to_edge)
    };

    load_edge_annotation(
        path,
        update,
        &pre_to_component,
        &pre_to_edge,
        id_to_node_name,
        is_annis_33,
        progress_callback,
    )?;

    Ok(())
}

fn postgresql_import_reader(path: &Path) -> std::result::Result<csv::Reader<File>, csv::Error> {
    csv::ReaderBuilder::new()
        .has_headers(false)
        .delimiter(b'\t')
        .quote(0) // effectivly disable quoting
        .from_path(path)
}

fn get_field_str(record: &csv::StringRecord, i: usize) -> Option<String> {
    if let Some(r) = record.get(i) {
        // replace some known escape sequences
        return Some(
            r.replace("\\t", "\t")
                .replace("\\'", "'")
                .replace("\\\\", "\\"),
        );
    }
    None
}

fn parse_corpus_tab<F>(
    path: &PathBuf,
    is_annis_33: bool,
    progress_callback: &F,
) -> Result<ParsedCorpusTable>
where
    F: Fn(&str) -> (),
{
    let mut corpus_tab_path = PathBuf::from(path);
    corpus_tab_path.push(if is_annis_33 {
        "corpus.annis"
    } else {
        "corpus.tab"
    });

    progress_callback(&format!(
        "loading {}",
        corpus_tab_path.to_str().unwrap_or_default()
    ));

    let mut toplevel_corpus_name: Option<String> = None;
    let mut corpus_by_preorder = BTreeMap::new();
    let mut corpus_id_to_name = BTreeMap::new();

    let mut corpus_tab_csv = postgresql_import_reader(corpus_tab_path.as_path())?;

    for result in corpus_tab_csv.records() {
        let line = result?;

        let id = line.get(0).ok_or("Missing column")?.parse::<u32>()?;
        let name = get_field_str(&line, 1).ok_or("Missing column")?;
        let type_str = get_field_str(&line, 2).ok_or("Missing column")?;
        let pre_order = line.get(4).ok_or("Missing column")?.parse::<u32>()?;

        corpus_id_to_name.insert(id, name.clone());
        if type_str == "CORPUS" && pre_order == 0 {
            toplevel_corpus_name = Some(name);
            corpus_by_preorder.insert(pre_order, id);
        } else if type_str == "DOCUMENT" {
            // TODO: do not only add documents but also sub-corpora
            corpus_by_preorder.insert(pre_order, id);
        }
    }

    let toplevel_corpus_name = toplevel_corpus_name.ok_or("Toplevel corpus name not found")?;
    Ok(ParsedCorpusTable {
        toplevel_corpus_name,
        corpus_by_preorder,
        corpus_id_to_name,
    })
}

fn parse_text_tab<F>(
    path: &PathBuf,
    is_annis_33: bool,
    progress_callback: &F,
) -> Result<HashMap<TextKey, Text>>
where
    F: Fn(&str) -> (),
{
    let mut text_tab_path = PathBuf::from(path);
    text_tab_path.push(if is_annis_33 {
        "text.annis"
    } else {
        "text.tab"
    });

    progress_callback(&format!(
        "loading {}",
        text_tab_path.to_str().unwrap_or_default()
    ));

    let mut texts: HashMap<TextKey, Text> = HashMap::default();

    let mut text_tab_csv = postgresql_import_reader(text_tab_path.as_path())?;

    for result in text_tab_csv.records() {
        let line = result?;

        let id = line
            .get(if is_annis_33 { 1 } else { 0 })
            .ok_or("Missing column")?
            .parse::<u32>()?;
        let name = get_field_str(&line, if is_annis_33 { 2 } else { 1 }).ok_or("Missing column")?;

        let corpus_ref = if is_annis_33 {
            Some(line.get(0).ok_or("Missing column")?.parse::<u32>()?)
        } else {
            None
        };
        let key = TextKey { id, corpus_ref };
        texts.insert(key.clone(), Text { name });
    }

    Ok(texts)
}

fn calculate_automatic_token_order<F>(
    update: &mut GraphUpdate,
    token_by_index: &BTreeMap<TextProperty, NodeID>,
    id_to_node_name: &FxHashMap<NodeID, String>,
    progress_callback: F,
) -> Result<()>
where
    F: Fn(&str) -> (),
{
    // TODO: cleanup, better variable naming
    // iterate over all token by their order, find the nodes with the same
    // text coverage (either left or right) and add explicit Ordering edge

    progress_callback("calculating the automatically generated Ordering edges");

    let mut last_textprop: Option<TextProperty> = None;
    let mut last_token: Option<NodeID> = None;

    for (current_textprop, current_token) in token_by_index {
        // if the last token/text value is valid and we are still in the same text
        if let (Some(last_token), Some(last_textprop)) = (last_token, last_textprop) {
            if last_textprop.corpus_id == current_textprop.corpus_id
                && last_textprop.text_id == current_textprop.text_id
                && last_textprop.segmentation == current_textprop.segmentation
            {
                // we are still in the same text, add ordering between token
                update.add_event(UpdateEvent::AddEdge {
                    source_node: id_to_node_name
                        .get(&last_token)
                        .ok_or("Missing node name")?
                        .clone(),
                    target_node: id_to_node_name
                        .get(current_token)
                        .ok_or("Missing node name")?
                        .clone(),
                    layer: ANNIS_NS.to_owned(),
                    component_type: ComponentType::Ordering.to_string(),
                    component_name: current_textprop.segmentation.clone(),
                });
            }
        } // end if same text

        // update the iterator and other variables
        last_textprop = Some(current_textprop.clone());
        last_token = Some(*current_token);
    } // end for each token

    Ok(())
}

fn add_automatic_cov_edge_for_node(
    update: &mut GraphUpdate,
    n: NodeID,
    left_pos: TextProperty,
    right_pos: TextProperty,
    textpos_table: &TextPosTable,
    id_to_node_name: &FxHashMap<NodeID, String>,
    text_coverage_containers: &UnionEdgeContainer,
) -> Result<()> {
    // find left/right aligned basic token
    let left_aligned_tok = textpos_table
        .token_by_left_textpos
        .get(&left_pos)
        .ok_or_else(|| format!("Can't get left-aligned token for node {}", n,));
    let right_aligned_tok = textpos_table
        .token_by_right_textpos
        .get(&right_pos)
        .ok_or_else(|| format!("Can't get right-aligned token for node {}", n,));

    // If only one of the aligned token is missing, use it for both sides, this is consistent with
    // the relANNIS import of ANNIS3
    let left_aligned_tok = if let Ok(left_aligned_tok) = left_aligned_tok {
        left_aligned_tok
    } else {
        right_aligned_tok.clone()?
    };
    let right_aligned_tok = if let Ok(right_aligned_tok) = right_aligned_tok {
        right_aligned_tok
    } else {
        left_aligned_tok
    };

    let left_tok_pos = textpos_table
        .token_to_index
        .get(&left_aligned_tok)
        .ok_or_else(|| {
            format!(
                "Can't get position of left-aligned token {}",
                left_aligned_tok
            )
        })?;
    let right_tok_pos = textpos_table
        .token_to_index
        .get(&right_aligned_tok)
        .ok_or_else(|| {
            format!(
                "Can't get position of right-aligned token {}",
                right_aligned_tok
            )
        })?;

    for i in left_tok_pos.val..(right_tok_pos.val + 1) {
        let tok_idx = TextProperty {
            segmentation: String::default(),
            corpus_id: left_tok_pos.corpus_id,
            text_id: left_tok_pos.text_id,
            val: i,
        };
        let tok_id = textpos_table
            .token_by_index
            .get(&tok_idx)
            .ok_or_else(|| format!("Can't get token ID for position {:?}", tok_idx))?;
        if n != *tok_id {
            // only add edge of no other coverage edge exists
            let existing_outgoing_cov: Vec<NodeID> =
                text_coverage_containers.get_outgoing_edges(n).collect();
            if !existing_outgoing_cov.contains(tok_id) {
                let component_name = if existing_outgoing_cov.is_empty() {
                    // the node has no other coverage edges, use a neutral component
                    ""
                } else {
                    // this is an additional auto-generated coverage edge, mark it as such
                    "autogenerated-coverage"
                };

                update.add_event(UpdateEvent::AddEdge {
                    source_node: id_to_node_name.get(&n).ok_or("Missing node name")?.clone(),
                    target_node: id_to_node_name
                        .get(tok_id)
                        .ok_or("Missing node name")?
                        .clone(),
                    layer: ANNIS_NS.to_owned(),
                    component_type: ComponentType::Coverage.to_string(),
                    component_name: component_name.to_owned(),
                });
            }
        }
    }

    Ok(())
}

fn calculate_automatic_coverage_edges<F>(
    update: &mut GraphUpdate,
    db: &Graph,
    textpos_table: &TextPosTable,
    id_to_node_name: &FxHashMap<NodeID, String>,
    progress_callback: &F,
) -> Result<()>
where
    F: Fn(&str) -> (),
{
    // add explicit coverage edges for each node in the special annis namespace coverage component
    progress_callback("calculating the automatically generated Coverage edges");

    let other_coverage_gs: Vec<&EdgeContainer> = db
        .get_all_components(Some(ComponentType::Coverage), None)
        .into_iter()
        .filter_map(|c| db.get_graphstorage_as_ref(&c))
        .map(|gs| gs.as_edgecontainer())
        .collect();

    let text_coverage_containers = UnionEdgeContainer::new(other_coverage_gs);

    for (n, textprop) in textpos_table.node_to_left.iter() {
        if textprop.segmentation == "" {
            if !textpos_table.token_to_index.contains_key(&n) {
                let left_pos = TextProperty {
                    segmentation: String::from(""),
                    corpus_id: textprop.corpus_id,
                    text_id: textprop.text_id,
                    val: textprop.val,
                };
                let right_pos = textpos_table
                    .node_to_right
                    .get(&n)
                    .ok_or_else(|| format!("Can't get right position of node {}", n))?;
                let right_pos = TextProperty {
                    segmentation: String::from(""),
                    corpus_id: textprop.corpus_id,
                    text_id: textprop.text_id,
                    val: right_pos.val,
                };

                if let Err(e) = add_automatic_cov_edge_for_node(
                    update,
                    *n,
                    left_pos,
                    right_pos,
                    textpos_table,
                    id_to_node_name,
                    &text_coverage_containers,
                ) {
                    // output a warning but do not fail
                    warn!(
                        "Adding coverage edges (connects spans with tokens) failed: {}",
                        e
                    )
                }
            } // end if not a token
        }
    }

    Ok(())
}

fn load_node_tab<F>(
    path: &PathBuf,
    update: &mut GraphUpdate,
    corpus_id_to_name: &BTreeMap<u32, String>,
    toplevel_corpus_name: &str,
    is_annis_33: bool,
    progress_callback: &F,
) -> Result<(
    MultiMap<TextKey, NodeID>,
    BTreeMap<NodeID, String>,
    FxHashMap<NodeID, String>,
    TextPosTable,
)>
where
    F: Fn(&str) -> (),
{
    let mut nodes_by_text: MultiMap<TextKey, NodeID> = MultiMap::new();
    let mut missing_seg_span: BTreeMap<NodeID, String> = BTreeMap::new();
    let mut id_to_node_name: FxHashMap<NodeID, String> = FxHashMap::default();

    let mut node_tab_path = PathBuf::from(path);
    node_tab_path.push(if is_annis_33 {
        "node.annis"
    } else {
        "node.tab"
    });

    progress_callback(&format!(
        "loading {}",
        node_tab_path.to_str().unwrap_or_default()
    ));

    // map the "left" value to the nodes it belongs to
    let mut left_to_node: MultiMap<TextProperty, NodeID> = MultiMap::new();
    // map the "right" value to the nodes it belongs to
    let mut right_to_node: MultiMap<TextProperty, NodeID> = MultiMap::new();

    // maps a character position to it's token
    let mut textpos_table = TextPosTable {
        token_by_left_textpos: BTreeMap::new(),
        token_by_right_textpos: BTreeMap::new(),
        node_to_left: BTreeMap::new(),
        node_to_right: BTreeMap::new(),
        token_by_index: BTreeMap::new(),
        token_to_index: BTreeMap::new(),
    };

    // start "scan all lines" visibility block
    {
        let mut node_tab_csv = postgresql_import_reader(node_tab_path.as_path())?;

        for result in node_tab_csv.records() {
            let line = result?;

            let node_nr = line.get(0).ok_or("Missing column")?.parse::<NodeID>()?;
            let has_segmentations = is_annis_33 || line.len() > 10;
            let token_index_raw = line.get(7).ok_or("Missing column")?;
            let text_id = line.get(1).ok_or("Missing column")?.parse::<u32>()?;
            let corpus_id = line.get(2).ok_or("Missing column")?.parse::<u32>()?;
            let layer = get_field_str(&line, 3).ok_or("Missing column")?;
            let node_name = get_field_str(&line, 4).ok_or("Missing column")?;

            nodes_by_text.insert(
                TextKey {
                    corpus_ref: Some(corpus_id),
                    id: text_id,
                },
                node_nr,
            );

            let doc_name = corpus_id_to_name
                .get(&corpus_id)
                .ok_or_else(|| format!("Document with ID {} missing", corpus_id))?;

            let node_qname = format!("{}/{}#{}", toplevel_corpus_name, doc_name, node_name);
            update.add_event(UpdateEvent::AddNode {
                node_name: node_qname.clone(),
                node_type: "node".to_owned(),
            });
            id_to_node_name.insert(node_nr, node_qname.clone());

            if !layer.is_empty() && layer != "NULL" {
                update.add_event(UpdateEvent::AddNodeLabel {
                    node_name: node_qname.clone(),
                    anno_ns: ANNIS_NS.to_owned(),
                    anno_name: "layer".to_owned(),
                    anno_value: layer,
                });
            }

            // Use left/right token columns for relANNIS 3.3 and the left/right character column otherwise.
            // For some malformed corpora, the token coverage information is more robust and guaranties that a node is
            // only left/right aligned to a single token.
            let left_column = if is_annis_33 { 8 } else { 5 };
            let right_column = if is_annis_33 { 9 } else { 6 };

            let left_val = line
                .get(left_column)
                .ok_or("Missing column")?
                .parse::<u32>()?;
            let left = TextProperty {
                segmentation: String::from(""),
                val: left_val,
                corpus_id,
                text_id,
            };
            let right_val = line
                .get(right_column)
                .ok_or("Missing column")?
                .parse::<u32>()?;
            let right = TextProperty {
                segmentation: String::from(""),
                val: right_val,
                corpus_id,
                text_id,
            };
            left_to_node.insert(left.clone(), node_nr);
            right_to_node.insert(right.clone(), node_nr);
            textpos_table.node_to_left.insert(node_nr, left.clone());
            textpos_table.node_to_right.insert(node_nr, right.clone());

            if token_index_raw != "NULL" {
                let span = if has_segmentations {
                    get_field_str(&line, 12).ok_or("Missing column")?
                } else {
                    get_field_str(&line, 9).ok_or("Missing column")?
                };

                update.add_event(UpdateEvent::AddNodeLabel {
                    node_name: node_qname,
                    anno_ns: ANNIS_NS.to_owned(),
                    anno_name: TOK.to_owned(),
                    anno_value: span,
                });

                let index = TextProperty {
                    segmentation: String::from(""),
                    val: token_index_raw.parse::<u32>()?,
                    text_id,
                    corpus_id,
                };
                textpos_table.token_by_index.insert(index.clone(), node_nr);
                textpos_table.token_to_index.insert(node_nr, index);
                textpos_table.token_by_left_textpos.insert(left, node_nr);
                textpos_table.token_by_right_textpos.insert(right, node_nr);
            } else if has_segmentations {
                let segmentation_name = if is_annis_33 {
                    get_field_str(&line, 11).ok_or("Missing column")?
                } else {
                    get_field_str(&line, 8).ok_or("Missing column")?
                };

                if segmentation_name != "NULL" {
                    let seg_index = if is_annis_33 {
                        line.get(10).ok_or("Missing column")?.parse::<u32>()?
                    } else {
                        line.get(9).ok_or("Missing column")?.parse::<u32>()?
                    };

                    if is_annis_33 {
                        // directly add the span information
                        update.add_event(UpdateEvent::AddNodeLabel {
                            node_name: node_qname,
                            anno_ns: ANNIS_NS.to_owned(),
                            anno_name: TOK.to_owned(),
                            anno_value: get_field_str(&line, 12).ok_or("Missing column")?,
                        });
                    } else {
                        // we need to get the span information from the node_annotation file later
                        missing_seg_span.insert(node_nr, segmentation_name.clone());
                    }
                    // also add the specific segmentation index
                    let index = TextProperty {
                        segmentation: segmentation_name,
                        val: seg_index,
                        corpus_id,
                        text_id,
                    };
                    textpos_table.token_by_index.insert(index, node_nr);
                } // end if node has segmentation info
            } // endif if check segmentations
        }
    } // end "scan all lines" visibility block

    if !textpos_table.token_by_index.is_empty() {
        calculate_automatic_token_order(
            update,
            &textpos_table.token_by_index,
            &id_to_node_name,
            progress_callback,
        )?;
    } // end if token_by_index not empty

    Ok((
        nodes_by_text,
        missing_seg_span,
        id_to_node_name,
        textpos_table,
    ))
}

fn load_node_anno_tab<F>(
    path: &PathBuf,
    update: &mut GraphUpdate,
    missing_seg_span: &BTreeMap<NodeID, String>,
    id_to_node_name: &FxHashMap<NodeID, String>,
    is_annis_33: bool,
    progress_callback: &F,
) -> Result<()>
where
    F: Fn(&str) -> (),
{
    let mut node_anno_tab_path = PathBuf::from(path);
    node_anno_tab_path.push(if is_annis_33 {
        "node_annotation.annis"
    } else {
        "node_annotation.tab"
    });

    progress_callback(&format!(
        "loading {}",
        node_anno_tab_path.to_str().unwrap_or_default()
    ));

    let mut node_anno_tab_csv = postgresql_import_reader(node_anno_tab_path.as_path())?;

    for result in node_anno_tab_csv.records() {
        let line = result?;

        let col_id = line.get(0).ok_or("Missing column")?;
        let node_id: NodeID = col_id.parse()?;
        let node_name = id_to_node_name.get(&node_id).ok_or("Missing node name")?;
        let col_ns = get_field_str(&line, 1).ok_or("Missing column")?;
        let col_name = get_field_str(&line, 2).ok_or("Missing column")?;
        let col_val = get_field_str(&line, 3).ok_or("Missing column")?;
        // we have to make some sanity checks
        if col_ns != "annis" || col_name != "tok" {
            let anno_val: String = if col_val == "NULL" {
                // use an "invalid" string so it can't be found by its value, but only by its annotation name
                std::char::MAX.to_string()
            } else {
                col_val
            };

            update.add_event(UpdateEvent::AddNodeLabel {
                node_name: node_name.clone(),
                anno_ns: col_ns,
                anno_name: col_name,
                anno_value: anno_val.clone(),
            });

            // add all missing span values from the annotation, but don't add NULL values
            if let Some(seg) = missing_seg_span.get(&node_id) {
                if seg == &get_field_str(&line, 2).ok_or("Missing column")?
                    && get_field_str(&line, 3).ok_or("Missing column")? != "NULL"
                {
                    update.add_event(UpdateEvent::AddNodeLabel {
                        node_name: node_name.clone(),
                        anno_ns: ANNIS_NS.to_owned(),
                        anno_name: TOK.to_owned(),
                        anno_value: anno_val,
                    });
                }
            }
        }
    }

    Ok(())
}

fn load_component_tab<F>(
    path: &PathBuf,
    is_annis_33: bool,
    progress_callback: &F,
) -> Result<BTreeMap<u32, Component>>
where
    F: Fn(&str) -> (),
{
    let mut component_tab_path = PathBuf::from(path);
    component_tab_path.push(if is_annis_33 {
        "component.annis"
    } else {
        "component.tab"
    });

    progress_callback(&format!(
        "loading {}",
        component_tab_path.to_str().unwrap_or_default()
    ));

    let mut component_by_id: BTreeMap<u32, Component> = BTreeMap::new();

    let mut component_tab_csv = postgresql_import_reader(component_tab_path.as_path())?;
    for result in component_tab_csv.records() {
        let line = result?;

        let cid: u32 = line.get(0).ok_or("Missing column")?.parse()?;
        let col_type = get_field_str(&line, 1).ok_or("Missing column")?;
        if col_type != "NULL" {
            let layer = get_field_str(&line, 2).ok_or("Missing column")?;
            let name = get_field_str(&line, 3).ok_or("Missing column")?;
            let name = if name == "NULL" {
                String::from("")
            } else {
                name
            };
            let ctype = component_type_from_short_name(&col_type)?;
            component_by_id.insert(cid, Component { ctype, layer, name });
        }
    }
    Ok(component_by_id)
}

fn load_nodes<F>(
    path: &PathBuf,
    update: &mut GraphUpdate,
    corpus_id_to_name: &BTreeMap<u32, String>,
    toplevel_corpus_name: &str,
    is_annis_33: bool,
    progress_callback: &F,
) -> Result<(
    MultiMap<TextKey, NodeID>,
    FxHashMap<NodeID, String>,
    TextPosTable,
)>
where
    F: Fn(&str) -> (),
{
    let (nodes_by_text, missing_seg_span, id_to_node_name, textpos_table) = load_node_tab(
        path,
        update,
        corpus_id_to_name,
        toplevel_corpus_name,
        is_annis_33,
        progress_callback,
    )?;
    load_node_anno_tab(
        path,
        update,
        &missing_seg_span,
        &id_to_node_name,
        is_annis_33,
        progress_callback,
    )?;

    Ok((nodes_by_text, id_to_node_name, textpos_table))
}

fn load_rank_tab<F>(
    path: &PathBuf,
    update: &mut GraphUpdate,
    component_by_id: &BTreeMap<u32, Component>,
    id_to_node_name: &FxHashMap<NodeID, String>,
    is_annis_33: bool,
    progress_callback: &F,
) -> Result<(BTreeMap<u32, Component>, BTreeMap<u32, Edge>)>
where
    F: Fn(&str) -> (),
{
    let mut rank_tab_path = PathBuf::from(path);
    rank_tab_path.push(if is_annis_33 {
        "rank.annis"
    } else {
        "rank.tab"
    });

    progress_callback(&format!(
        "loading {}",
        rank_tab_path.to_str().unwrap_or_default()
    ));

    let mut rank_tab_csv = postgresql_import_reader(rank_tab_path.as_path())?;

    let pos_node_ref = if is_annis_33 { 3 } else { 2 };
    let pos_component_ref = if is_annis_33 { 4 } else { 3 };
    let pos_parent = if is_annis_33 { 5 } else { 4 };

    // first run: collect all pre-order values for a node
    let mut pre_to_node_id: BTreeMap<u32, NodeID> = BTreeMap::new();
    for result in rank_tab_csv.records() {
        let line = result?;
        let pre: u32 = line.get(0).ok_or("Missing column")?.parse()?;
        let node_id: NodeID = line.get(pos_node_ref).ok_or("Missing column")?.parse()?;
        pre_to_node_id.insert(pre, node_id);
    }

    let mut pre_to_component: BTreeMap<u32, Component> = BTreeMap::new();
    let mut pre_to_edge: BTreeMap<u32, Edge> = BTreeMap::new();
    // second run: get the actual edges
    let mut rank_tab_csv = postgresql_import_reader(rank_tab_path.as_path())?;

    for result in rank_tab_csv.records() {
        let line = result?;

        let parent_as_str = line.get(pos_parent).ok_or("Missing column")?;
        if parent_as_str != "NULL" {
            let parent: u32 = parent_as_str.parse()?;
            if let Some(source) = pre_to_node_id.get(&parent) {
                // find the responsible edge database by the component ID
                let component_ref: u32 = line
                    .get(pos_component_ref)
                    .ok_or("Missing column")?
                    .parse()?;
                if let Some(c) = component_by_id.get(&component_ref) {
                    let target: NodeID = line.get(pos_node_ref).ok_or("Missing column")?.parse()?;

                    update.add_event(UpdateEvent::AddEdge {
                        source_node: id_to_node_name
                            .get(&source)
                            .ok_or("Missing node name")?
                            .to_owned(),
                        target_node: id_to_node_name
                            .get(&target)
                            .ok_or("Missing node name")?
                            .to_owned(),
                        layer: c.layer.clone(),
                        component_type: c.ctype.to_string(),
                        component_name: c.name.clone(),
                    });

                    let pre: u32 = line.get(0).ok_or("Missing column")?.parse()?;

                    let e = Edge {
                        source: *source,
                        target,
                    };

                    pre_to_edge.insert(pre, e);
                    pre_to_component.insert(pre, c.clone());
                }
            }
        }
    }

    Ok((pre_to_component, pre_to_edge))
}

fn load_edge_annotation<F>(
    path: &PathBuf,
    update: &mut GraphUpdate,
    pre_to_component: &BTreeMap<u32, Component>,
    pre_to_edge: &BTreeMap<u32, Edge>,
    id_to_node_name: &FxHashMap<NodeID, String>,
    is_annis_33: bool,
    progress_callback: &F,
) -> Result<()>
where
    F: Fn(&str) -> (),
{
    let mut edge_anno_tab_path = PathBuf::from(path);
    edge_anno_tab_path.push(if is_annis_33 {
        "edge_annotation.annis"
    } else {
        "edge_annotation.tab"
    });

    progress_callback(&format!(
        "loading {}",
        edge_anno_tab_path.to_str().unwrap_or_default()
    ));

    let mut edge_anno_tab_csv = postgresql_import_reader(edge_anno_tab_path.as_path())?;

    for result in edge_anno_tab_csv.records() {
        let line = result?;

        let pre: u32 = line.get(0).ok_or("Missing column")?.parse()?;
        if let Some(c) = pre_to_component.get(&pre) {
            if let Some(e) = pre_to_edge.get(&pre) {
                let ns = get_field_str(&line, 1).ok_or("Missing column")?;
                let name = get_field_str(&line, 2).ok_or("Missing column")?;
                let val = get_field_str(&line, 3).ok_or("Missing column")?;

                update.add_event(UpdateEvent::AddEdgeLabel {
                    source_node: id_to_node_name
                        .get(&e.source)
                        .ok_or("Missing node name")?
                        .to_owned(),
                    target_node: id_to_node_name
                        .get(&e.target)
                        .ok_or("Missing node name")?
                        .to_owned(),
                    layer: c.layer.clone(),
                    component_type: c.ctype.to_string(),
                    component_name: c.name.clone(),
                    anno_ns: ns,
                    anno_name: name,
                    anno_value: val,
                });
            }
        }
    }

    Ok(())
}

fn load_corpus_annotation<F>(
    path: &PathBuf,
    is_annis_33: bool,
    progress_callback: &F,
) -> Result<MultiMap<u32, Annotation>>
where
    F: Fn(&str) -> (),
{
    let mut corpus_id_to_anno = MultiMap::new();

    let mut corpus_anno_tab_path = PathBuf::from(path);
    corpus_anno_tab_path.push(if is_annis_33 {
        "corpus_annotation.annis"
    } else {
        "corpus_annotation.tab"
    });

    progress_callback(&format!(
        "loading {}",
        corpus_anno_tab_path.to_str().unwrap_or_default()
    ));

    let mut corpus_anno_tab_csv = postgresql_import_reader(corpus_anno_tab_path.as_path())?;

    for result in corpus_anno_tab_csv.records() {
        let line = result?;

        let id = line.get(0).ok_or("Missing column")?.parse()?;
        let ns = get_field_str(&line, 1).ok_or("Missing column")?;
        let ns = if ns == "NULL" { String::default() } else { ns };
        let name = get_field_str(&line, 2).ok_or("Missing column")?;
        let val = get_field_str(&line, 3).ok_or("Missing column")?;

        let anno = Annotation {
            key: AnnoKey { ns, name },
            val,
        };

        corpus_id_to_anno.insert(id, anno);
    }

    Ok(corpus_id_to_anno)
}

fn add_subcorpora(
    update: &mut GraphUpdate,
    corpus_table: &ParsedCorpusTable,
    nodes_by_text: &MultiMap<TextKey, NodeID>,
    texts: &HashMap<TextKey, Text>,
    corpus_id_to_annos: &MultiMap<u32, Annotation>,
    id_to_node_name: &FxHashMap<NodeID, String>,
    is_annis_33: bool,
) -> Result<()> {
    // add the toplevel corpus as node
    {
        update.add_event(UpdateEvent::AddNode {
            node_name: corpus_table.toplevel_corpus_name.to_owned(),
            node_type: "corpus".to_owned(),
        });

        // add all metadata for the top-level corpus node
        if let Some(cid) = corpus_table.corpus_by_preorder.get(&0) {
            if let Some(anno_vec) = corpus_id_to_annos.get_vec(cid) {
                for anno in anno_vec {
                    update.add_event(UpdateEvent::AddNodeLabel {
                        node_name: corpus_table.toplevel_corpus_name.to_owned(),
                        anno_ns: anno.key.ns.clone(),
                        anno_name: anno.key.name.clone(),
                        anno_value: anno.val.clone(),
                    });
                }
            }
        }
    }

    // add all subcorpora/documents (start with the largest pre-order)
    for (pre, corpus_id) in corpus_table.corpus_by_preorder.iter().rev() {
        if *pre != 0 {
            let corpus_name = corpus_table
                .corpus_id_to_name
                .get(corpus_id)
                .ok_or_else(|| format!("Can't get name for corpus with ID {}", corpus_id))?;
            let subcorpus_full_name =
                format!("{}/{}", corpus_table.toplevel_corpus_name, corpus_name);

            // add a basic node labels for the new (sub-) corpus/document
            update.add_event(UpdateEvent::AddNode {
                node_name: subcorpus_full_name.clone(),
                node_type: "corpus".to_owned(),
            });
            update.add_event(UpdateEvent::AddNodeLabel {
                node_name: subcorpus_full_name.clone(),
                anno_ns: ANNIS_NS.to_owned(),
                anno_name: "doc".to_owned(),
                anno_value: corpus_name.to_owned(),
            });

            // add all metadata for the document node
            if let Some(anno_vec) = corpus_id_to_annos.get_vec(&corpus_id) {
                for anno in anno_vec {
                    update.add_event(UpdateEvent::AddNodeLabel {
                        node_name: subcorpus_full_name.clone(),
                        anno_ns: anno.key.ns.clone(),
                        anno_name: anno.key.name.clone(),
                        anno_value: anno.val.clone(),
                    });
                }
            }
            // add an edge from the document (or sub-corpus) to the top-level corpus
            update.add_event(UpdateEvent::AddEdge {
                source_node: subcorpus_full_name.clone(),
                target_node: corpus_table.toplevel_corpus_name.to_owned(),
                layer: ANNIS_NS.to_owned(),
                component_type: ComponentType::PartOfSubcorpus.to_string(),
                component_name: String::default(),
            });
        } // end if not toplevel corpus
    } // end for each document/sub-corpus

    // add a node for each text and the connection between all sub-nodes of the text
    for text_key in nodes_by_text.keys() {
        // add text node (including its name)
        let text_name: Option<String> = if is_annis_33 {
            // corpus_ref is included in the text.annis
            texts.get(text_key).map(|k| k.name.clone())
        } else {
            // create a text key without corpus_ref, since it is not in the parsed result
            let new_text_key = TextKey {
                id: text_key.id,
                corpus_ref: None,
            };
            texts.get(&new_text_key).map(|k| k.name.clone())
        };
        if let (Some(text_name), Some(corpus_ref)) = (text_name, text_key.corpus_ref) {
            let corpus_name = corpus_table
                .corpus_id_to_name
                .get(&corpus_ref)
                .ok_or_else(|| format!("Can't get name for corpus with ID {}", corpus_ref))?;
            let subcorpus_full_name =
                format!("{}/{}", corpus_table.toplevel_corpus_name, corpus_name);
            let text_full_name = format!(
                "{}/{}#{}",
                corpus_table.toplevel_corpus_name, corpus_name, text_name
            );

            update.add_event(UpdateEvent::AddNode {
                node_name: text_full_name.clone(),
                node_type: "datasource".to_owned(),
            });

            // add an edge from the text to the document
            update.add_event(UpdateEvent::AddEdge {
                source_node: text_full_name.clone(),
                target_node: subcorpus_full_name,
                layer: ANNIS_NS.to_owned(),
                component_type: ComponentType::PartOfSubcorpus.to_string(),
                component_name: String::default(),
            });

            // find all nodes belonging to this text and add a relation
            if let Some(n_vec) = nodes_by_text.get_vec(text_key) {
                for n in n_vec {
                    update.add_event(UpdateEvent::AddEdge {
                        source_node: id_to_node_name.get(n).ok_or("Missing node name")?.clone(),
                        target_node: text_full_name.clone(),
                        layer: ANNIS_NS.to_owned(),
                        component_type: ComponentType::PartOfSubcorpus.to_string(),
                        component_name: String::default(),
                    });
                }
            }
        }
    } // end for each text

    Ok(())
}

fn component_type_from_short_name(short_type: &str) -> Result<ComponentType> {
    match short_type {
        "c" => Ok(ComponentType::Coverage),
        "d" => Ok(ComponentType::Dominance),
        "p" => Ok(ComponentType::Pointing),
        "o" => Ok(ComponentType::Ordering),
        _ => Err(format!("Invalid component type short name '{}'", short_type).into()),
    }
}
