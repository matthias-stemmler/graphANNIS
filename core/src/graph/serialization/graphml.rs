use crate::{
    annostorage::{Match, ValueSearch},
    errors::{GraphAnnisCoreError, Result},
    graph::{
        update::{GraphUpdate, UpdateEvent},
        Graph, ANNIS_NS, NODE_NAME, NODE_NAME_KEY, NODE_TYPE, NODE_TYPE_KEY,
    },
    types::{AnnoKey, Annotation, Component, ComponentType, Edge},
    util::{join_qname, split_qname},
};
use itertools::Itertools;
use quick_xml::{
    events::{
        attributes::Attributes, BytesCData, BytesDecl, BytesEnd, BytesStart, BytesText, Event,
    },
    Reader, Writer,
};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, HashMap},
    io::{BufReader, BufWriter, Read, Write},
    str::FromStr,
};

fn write_annotation_keys<CT: ComponentType, W: std::io::Write>(
    graph: &Graph<CT>,
    has_graph_configuration: bool,
    sorted: bool,
    writer: &mut Writer<W>,
) -> Result<BTreeMap<AnnoKey, String>> {
    let mut key_id_mapping = BTreeMap::new();
    let mut id_counter = 0;

    if has_graph_configuration {
        let new_id = format!("k{}", id_counter);
        id_counter += 1;

        let mut key_start = BytesStart::new("key");
        key_start.push_attribute(("id", new_id.as_str()));
        key_start.push_attribute(("for", "graph"));
        key_start.push_attribute(("attr.name", "configuration"));
        key_start.push_attribute(("attr.type", "string"));

        writer.write_event(Event::Empty(key_start))?;
    }

    // Create node annotation keys
    let mut anno_keys = graph.get_node_annos().annotation_keys()?;
    if sorted {
        anno_keys.sort_unstable();
    }
    for key in anno_keys {
        if (key.ns != ANNIS_NS || key.name != NODE_NAME) && !key_id_mapping.contains_key(&key) {
            let new_id = format!("k{}", id_counter);
            id_counter += 1;

            let qname = join_qname(&key.ns, &key.name);

            let mut key_start = BytesStart::new("key");
            key_start.push_attribute(("id", new_id.as_str()));
            key_start.push_attribute(("for", "node"));
            key_start.push_attribute(("attr.name", qname.as_str()));
            key_start.push_attribute(("attr.type", "string"));

            writer.write_event(Event::Empty(key_start))?;

            key_id_mapping.insert(key, new_id);
        }
    }

    // Create edge annotation keys for all components, but skip auto-generated ones
    let autogenerated_components: BTreeSet<Component<CT>> =
        CT::update_graph_index_components(graph)
            .into_iter()
            .collect();
    let mut all_components = graph.get_all_components(None, None);
    if sorted {
        all_components.sort_unstable();
    }
    for c in all_components {
        if !autogenerated_components.contains(&c) {
            if let Some(gs) = graph.get_graphstorage(&c) {
                for key in gs.get_anno_storage().annotation_keys()? {
                    #[allow(clippy::map_entry)]
                    if !key_id_mapping.contains_key(&key) {
                        let new_id = format!("k{}", id_counter);
                        id_counter += 1;

                        let qname = join_qname(&key.ns, &key.name);

                        let mut key_start = BytesStart::new("key");
                        key_start.push_attribute(("id", new_id.as_str()));
                        key_start.push_attribute(("for", "node"));
                        key_start.push_attribute(("attr.name", qname.as_str()));
                        key_start.push_attribute(("attr.type", "string"));

                        writer.write_event(Event::Empty(key_start))?;

                        key_id_mapping.insert(key, new_id);
                    }
                }
            }
        }
    }

    Ok(key_id_mapping)
}

fn write_data<W: std::io::Write>(
    anno: Annotation,
    writer: &mut Writer<W>,
    key_id_mapping: &BTreeMap<AnnoKey, String>,
) -> Result<()> {
    let mut data_start = BytesStart::new("data");

    let key_id = key_id_mapping
        .get(&anno.key)
        .ok_or_else(|| GraphAnnisCoreError::GraphMLMissingAnnotationKey(anno.key.clone()))?;

    data_start.push_attribute(("key", key_id.as_str()));
    writer.write_event(Event::Start(data_start))?;
    // Add the annotation value as internal text node
    writer.write_event(Event::Text(BytesText::new(&anno.val)))?;
    writer.write_event(Event::End(BytesEnd::new("data")))?;

    Ok(())
}

fn compare_results<T: Ord>(a: &Result<T>, b: &Result<T>) -> Ordering {
    if let (Ok(a), Ok(b)) = (a, b) {
        a.cmp(b)
    } else if a.is_err() {
        Ordering::Less
    } else if b.is_err() {
        Ordering::Greater
    } else {
        // Treat two errors as equal
        Ordering::Equal
    }
}

fn write_nodes<CT: ComponentType, W: std::io::Write>(
    graph: &Graph<CT>,
    writer: &mut Writer<W>,
    sorted: bool,
    key_id_mapping: &BTreeMap<AnnoKey, String>,
) -> Result<()> {
    let base_node_iterator =
        graph
            .get_node_annos()
            .exact_anno_search(Some(ANNIS_NS), NODE_TYPE, ValueSearch::Any);
    let node_iterator: Box<dyn Iterator<Item = Result<Match>>> = if sorted {
        let it = base_node_iterator.sorted_unstable_by(compare_results);
        Box::new(it)
    } else {
        Box::new(base_node_iterator)
    };

    for m in node_iterator {
        let m = m?;
        let mut node_start = BytesStart::new("node");

        if let Some(id) = graph
            .get_node_annos()
            .get_value_for_item(&m.node, &NODE_NAME_KEY)?
        {
            node_start.push_attribute(("id", id.as_ref()));
            let mut node_annotations = graph.get_node_annos().get_annotations_for_item(&m.node)?;
            if node_annotations.is_empty() {
                // Write an empty XML element without child nodes
                writer.write_event(Event::Empty(node_start))?;
            } else {
                writer.write_event(Event::Start(node_start))?;
                // Write all annotations of the node as "data" element, but sort
                // them using the internal annotation key (k0, k1, k2, etc.)
                node_annotations.sort_unstable_by_key(|anno| {
                    key_id_mapping
                        .get(&anno.key)
                        .map(|internal_key| internal_key.as_str())
                        .unwrap_or("")
                });

                for anno in node_annotations {
                    if anno.key.ns != ANNIS_NS || anno.key.name != NODE_NAME {
                        write_data(anno, writer, key_id_mapping)?;
                    }
                }
                writer.write_event(Event::End(BytesEnd::new("node")))?;
            }
        }
    }
    Ok(())
}

fn write_edges<CT: ComponentType, W: std::io::Write>(
    graph: &Graph<CT>,
    writer: &mut Writer<W>,
    sorted: bool,
    key_id_mapping: &BTreeMap<AnnoKey, String>,
) -> Result<()> {
    let mut edge_counter = 0;
    let autogenerated_components: BTreeSet<Component<CT>> =
        CT::update_graph_index_components(graph)
            .into_iter()
            .collect();

    let mut all_components = graph.get_all_components(None, None);
    if sorted {
        all_components.sort_unstable();
    }

    for c in all_components {
        // Create edge annotation keys for all components, but skip auto-generated ones
        if !autogenerated_components.contains(&c) {
            if let Some(gs) = graph.get_graphstorage(&c) {
                let source_nodes_iterator = if sorted {
                    Box::new(gs.source_nodes().sorted_unstable_by(compare_results))
                } else {
                    gs.source_nodes()
                };
                for source in source_nodes_iterator {
                    let source = source?;
                    if let Some(source_id) = graph
                        .get_node_annos()
                        .get_value_for_item(&source, &NODE_NAME_KEY)?
                    {
                        let target_nodes_iterator = if sorted {
                            Box::new(
                                gs.get_outgoing_edges(source)
                                    .sorted_unstable_by(compare_results),
                            )
                        } else {
                            gs.get_outgoing_edges(source)
                        };
                        for target in target_nodes_iterator {
                            let target = target?;
                            if let Some(target_id) = graph
                                .get_node_annos()
                                .get_value_for_item(&target, &NODE_NAME_KEY)?
                            {
                                let edge = Edge { source, target };

                                let mut edge_id = edge_counter.to_string();
                                edge_counter += 1;
                                edge_id.insert(0, 'e');

                                let mut edge_start = BytesStart::new("edge");
                                edge_start.push_attribute(("id", edge_id.as_str()));
                                edge_start.push_attribute(("source", source_id.as_ref()));
                                edge_start.push_attribute(("target", target_id.as_ref()));
                                // Use the "label" attribute as component type. This is consistent with how Neo4j interprets this non-standard attribute
                                edge_start.push_attribute(("label", c.to_string().as_ref()));

                                writer.write_event(Event::Start(edge_start))?;

                                // Write all annotations of the node as "data" element, but sort
                                // them using the internal annotation key (k0, k1, k2, etc.)
                                let mut edge_annotations =
                                    gs.get_anno_storage().get_annotations_for_item(&edge)?;
                                edge_annotations.sort_unstable_by_key(|anno| {
                                    key_id_mapping
                                        .get(&anno.key)
                                        .map(|internal_key| internal_key.as_str())
                                        .unwrap_or("")
                                });
                                for anno in edge_annotations {
                                    write_data(anno, writer, key_id_mapping)?;
                                }
                                writer.write_event(Event::End(BytesEnd::new("edge")))?;
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn export<CT: ComponentType, W: std::io::Write, F>(
    graph: &Graph<CT>,
    graph_configuration: Option<&str>,
    output: W,
    progress_callback: F,
) -> Result<()>
where
    F: Fn(&str),
{
    // Always buffer the output
    let output = BufWriter::new(output);
    let mut writer = Writer::new_with_indent(output, b' ', 4);

    // Add XML declaration
    let xml_decl = BytesDecl::new("1.0", Some("UTF-8"), None);
    writer.write_event(Event::Decl(xml_decl))?;

    // Always write the root element
    writer.write_event(Event::Start(BytesStart::new("graphml")))?;

    // Define all valid annotation ns/name pairs
    progress_callback("exporting all available annotation keys");
    let key_id_mapping =
        write_annotation_keys(graph, graph_configuration.is_some(), false, &mut writer)?;

    // We are writing a single graph
    let mut graph_start = BytesStart::new("graph");
    graph_start.push_attribute(("edgedefault", "directed"));
    // Add parse helper information to allow more efficient parsing
    graph_start.push_attribute(("parse.order", "nodesfirst"));
    graph_start.push_attribute(("parse.nodeids", "free"));
    graph_start.push_attribute(("parse.edgeids", "canonical"));

    writer.write_event(Event::Start(graph_start))?;

    // If graph configuration is given, add it as data element to the graph
    if let Some(config) = graph_configuration {
        let mut data_start = BytesStart::new("data");
        // This is always the first key ID
        data_start.push_attribute(("key", "k0"));
        writer.write_event(Event::Start(data_start))?;
        // Add the annotation value as internal text node
        writer.write_event(Event::CData(BytesCData::new(config)))?;
        writer.write_event(Event::End(BytesEnd::new("data")))?;
    }

    // Write out all nodes
    progress_callback("exporting nodes");
    write_nodes(graph, &mut writer, false, &key_id_mapping)?;

    // Write out all edges
    progress_callback("exporting edges");
    write_edges(graph, &mut writer, false, &key_id_mapping)?;

    writer.write_event(Event::End(BytesEnd::new("graph")))?;
    writer.write_event(Event::End(BytesEnd::new("graphml")))?;

    // Make sure to flush the buffered writer
    writer.into_inner().flush()?;

    Ok(())
}

/// Export the GraphML file and ensure a stable order of the XML elements.
///
/// This is slower than [`export`] but can e.g. be used in tests where the
/// output should always be the same.
pub fn export_stable_order<CT: ComponentType, W: std::io::Write, F>(
    graph: &Graph<CT>,
    graph_configuration: Option<&str>,
    output: W,
    progress_callback: F,
) -> Result<()>
where
    F: Fn(&str),
{
    // Always buffer the output
    let output = BufWriter::new(output);
    let mut writer = Writer::new_with_indent(output, b' ', 4);

    // Add XML declaration
    let xml_decl = BytesDecl::new("1.0", Some("UTF-8"), None);
    writer.write_event(Event::Decl(xml_decl))?;

    // Always write the root element
    writer.write_event(Event::Start(BytesStart::new("graphml")))?;

    // Define all valid annotation ns/name pairs
    progress_callback("exporting all available annotation keys");
    let key_id_mapping =
        write_annotation_keys(graph, graph_configuration.is_some(), true, &mut writer)?;

    // We are writing a single graph
    let mut graph_start = BytesStart::new("graph");
    graph_start.push_attribute(("edgedefault", "directed"));
    // Add parse helper information to allow more efficient parsing
    graph_start.push_attribute(("parse.order", "nodesfirst"));
    graph_start.push_attribute(("parse.nodeids", "free"));
    graph_start.push_attribute(("parse.edgeids", "canonical"));

    writer.write_event(Event::Start(graph_start))?;

    // If graph configuration is given, add it as data element to the graph
    if let Some(config) = graph_configuration {
        let mut data_start = BytesStart::new("data");
        // This is always the first key ID
        data_start.push_attribute(("key", "k0"));
        writer.write_event(Event::Start(data_start))?;
        // Add the annotation value as internal text node
        writer.write_event(Event::CData(BytesCData::new(config)))?;
        writer.write_event(Event::End(BytesEnd::new("data")))?;
    }

    // Write out all nodes
    progress_callback("exporting nodes");
    write_nodes(graph, &mut writer, true, &key_id_mapping)?;

    // Write out all edges
    progress_callback("exporting edges");
    write_edges(graph, &mut writer, true, &key_id_mapping)?;

    writer.write_event(Event::End(BytesEnd::new("graph")))?;
    writer.write_event(Event::End(BytesEnd::new("graphml")))?;

    // Make sure to flush the buffered writer
    writer.into_inner().flush()?;

    Ok(())
}

fn add_annotation_key(keys: &mut BTreeMap<String, AnnoKey>, attributes: Attributes) -> Result<()> {
    // resolve the ID to the fully qualified annotation name
    let mut id: Option<String> = None;
    let mut anno_key: Option<AnnoKey> = None;

    for att in attributes {
        let att = att?;

        let att_value = String::from_utf8_lossy(&att.value);

        match att.key.0 {
            b"id" => {
                id = Some(att_value.to_string());
            }
            b"attr.name" => {
                let (ns, name) = split_qname(att_value.as_ref());
                anno_key = Some(AnnoKey {
                    ns: ns.unwrap_or("").into(),
                    name: name.into(),
                });
            }
            _ => {}
        }
    }

    if let (Some(id), Some(anno_key)) = (id, anno_key) {
        keys.insert(id, anno_key);
    }
    Ok(())
}

fn add_node(
    node_updates: &mut GraphUpdate,
    current_node_id: &Option<String>,
    data: &mut HashMap<AnnoKey, String>,
) -> Result<()> {
    if let Some(node_name) = current_node_id {
        // Insert graph update for node
        let node_type = data
            .remove(&NODE_TYPE_KEY)
            .unwrap_or_else(|| "node".to_string());
        node_updates.add_event(UpdateEvent::AddNode {
            node_name: node_name.clone(),
            node_type,
        })?;
        // Add all remaining data entries as annotations
        for (key, value) in data.drain() {
            node_updates.add_event(UpdateEvent::AddNodeLabel {
                node_name: node_name.clone(),
                anno_ns: key.ns.into(),
                anno_name: key.name.into(),
                anno_value: value,
            })?;
        }
    }
    Ok(())
}

fn add_edge<CT: ComponentType>(
    edge_updates: &mut GraphUpdate,
    current_source_id: &Option<String>,
    current_target_id: &Option<String>,
    current_component: &Option<String>,
    data: &mut HashMap<AnnoKey, String>,
) -> Result<()> {
    if let (Some(source), Some(target), Some(component)) =
        (current_source_id, current_target_id, current_component)
    {
        // Insert graph update for this edge
        if let Ok(component) = Component::<CT>::from_str(component) {
            edge_updates.add_event(UpdateEvent::AddEdge {
                source_node: source.clone(),
                target_node: target.clone(),
                layer: component.layer.clone().into(),
                component_type: component.get_type().to_string(),
                component_name: component.name.clone().into(),
            })?;

            // Add all remaining data entries as annotations
            for (key, value) in data.drain() {
                edge_updates.add_event(UpdateEvent::AddEdgeLabel {
                    source_node: source.clone(),
                    target_node: target.clone(),
                    layer: component.layer.clone().into(),
                    component_type: component.get_type().to_string(),
                    component_name: component.name.clone().into(),
                    anno_ns: key.ns.into(),
                    anno_name: key.name.into(),
                    anno_value: value,
                })?;
            }
        }
    }
    Ok(())
}

fn read_graphml<CT: ComponentType, R: std::io::BufRead, F: Fn(&str)>(
    input: &mut R,
    node_updates: &mut GraphUpdate,
    edge_updates: &mut GraphUpdate,
    progress_callback: &F,
) -> Result<Option<String>> {
    let mut reader = Reader::from_reader(input);
    reader.expand_empty_elements(true);

    let mut keys = BTreeMap::new();

    let mut level = 0;
    let mut in_graph = false;
    let mut current_node_id: Option<String> = None;
    let mut current_data_key: Option<String> = None;
    let mut current_source_id: Option<String> = None;
    let mut current_target_id: Option<String> = None;
    let mut current_component: Option<String> = None;
    let mut current_data_value: Option<String> = None;
    let mut data: HashMap<AnnoKey, String> = HashMap::new();

    let mut config = None;

    let mut processed_updates = 0;

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(ref e) => {
                level += 1;

                match e.name().0 {
                    b"graph" => {
                        if level == 2 {
                            in_graph = true;
                        }
                    }
                    b"key" => {
                        if level == 2 {
                            add_annotation_key(&mut keys, e.attributes())?;
                        }
                    }
                    b"node" => {
                        if in_graph && level == 3 {
                            data.clear();
                            // Get the ID of this node
                            for att in e.attributes() {
                                let att = att?;
                                if att.key.0 == b"id" {
                                    current_node_id =
                                        Some(String::from_utf8_lossy(&att.value).to_string());
                                }
                            }
                        }
                    }
                    b"edge" => {
                        if in_graph && level == 3 {
                            data.clear();
                            // Get the source and target node IDs
                            for att in e.attributes() {
                                let att = att?;
                                if att.key.0 == b"source" {
                                    current_source_id =
                                        Some(String::from_utf8_lossy(&att.value).to_string());
                                } else if att.key.0 == b"target" {
                                    current_target_id =
                                        Some(String::from_utf8_lossy(&att.value).to_string());
                                } else if att.key.0 == b"label" {
                                    current_component =
                                        Some(String::from_utf8_lossy(&att.value).to_string());
                                }
                            }
                        }
                    }
                    b"data" => {
                        for att in e.attributes() {
                            let att = att?;
                            if att.key.0 == b"key" {
                                current_data_key =
                                    Some(String::from_utf8_lossy(&att.value).to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(t) => {
                if in_graph && level == 4 && current_data_key.is_some() {
                    current_data_value = Some(t.unescape()?.to_string());
                }
            }
            Event::CData(t) => {
                if let Some(current_data_key) = &current_data_key {
                    if in_graph && level == 3 && current_data_key == "k0" {
                        // This is the configuration content
                        config = Some(String::from_utf8_lossy(&t).to_string());
                    }
                }
            }
            Event::End(ref e) => {
                match e.name().0 {
                    b"graph" => {
                        in_graph = false;
                    }
                    b"node" => {
                        add_node(node_updates, &current_node_id, &mut data)?;
                        current_node_id = None;
                        processed_updates += 1;
                        if processed_updates % 1_000_000 == 0 {
                            progress_callback(&format!(
                                "Processed {} GraphML nodes and edges",
                                processed_updates
                            ));
                        }
                    }
                    b"edge" => {
                        add_edge::<CT>(
                            edge_updates,
                            &current_source_id,
                            &current_target_id,
                            &current_component,
                            &mut data,
                        )?;
                        current_source_id = None;
                        current_target_id = None;
                        current_component = None;
                        processed_updates += 1;
                        if processed_updates % 1_000_000 == 0 {
                            progress_callback(&format!(
                                "Processed {} GraphML nodes and edges",
                                processed_updates
                            ));
                        }
                    }
                    b"data" => {
                        if let Some(current_data_key) = current_data_key {
                            if let Some(anno_key) = keys.get(&current_data_key) {
                                // Copy all data attributes into our own map
                                if let Some(v) = current_data_value.take() {
                                    data.insert(anno_key.clone(), v);
                                } else {
                                    // If there is an end tag without any text
                                    // data event, the value exists but is
                                    // empty.
                                    data.insert(anno_key.clone(), String::default());
                                }
                            }
                        }

                        current_data_value = None;
                        current_data_key = None;
                    }
                    _ => {}
                }

                level -= 1;
            }
            Event::Eof => {
                break;
            }
            _ => {}
        }
        // Clear the buffer after each event
        buf.clear();
    }
    Ok(config)
}

pub fn import<CT: ComponentType, R: Read, F>(
    input: R,
    disk_based: bool,
    progress_callback: F,
) -> Result<(Graph<CT>, Option<String>)>
where
    F: Fn(&str),
{
    // Always buffer the read operations
    let mut input = BufReader::new(input);
    let mut g = Graph::with_default_graphstorages(disk_based)?;
    let mut updates = GraphUpdate::default();
    let mut edge_updates = GraphUpdate::default();

    // read in all nodes and edges, collecting annotation keys on the fly
    progress_callback("reading GraphML");
    let config = read_graphml::<CT, BufReader<R>, F>(
        &mut input,
        &mut updates,
        &mut edge_updates,
        &progress_callback,
    )?;

    // Append all edges updates after the node updates:
    // edges would not be added if the nodes they are referring do not exist
    progress_callback("merging generated events");
    for event in edge_updates.iter()? {
        let (_, event) = event?;
        updates.add_event(event)?;
    }

    progress_callback("applying imported changes");
    g.apply_update(&mut updates, &progress_callback)?;

    progress_callback("calculating node statistics");
    g.get_node_annos_mut().calculate_statistics()?;

    for c in g.get_all_components(None, None) {
        progress_callback(&format!("calculating statistics for component {}", c));
        g.calculate_component_statistics(&c)?;
        g.optimize_gs_impl(&c)?;
    }

    Ok((g, config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        graph::{GraphUpdate, DEFAULT_NS},
        types::DefaultComponentType,
    };
    use pretty_assertions::assert_eq;
    use std::borrow::Cow;

    const TEST_CONFIG: &str = r#"[some]
key = "<value>"

[some.another]
value = "test""#;

    #[test]
    fn export_graphml() {
        // Create a sample graph using the simple type
        let mut u = GraphUpdate::new();
        u.add_event(UpdateEvent::AddNode {
            node_name: "first_node".to_string(),
            node_type: "node".to_string(),
        })
        .unwrap();
        u.add_event(UpdateEvent::AddNode {
            node_name: "second_node".to_string(),
            node_type: "node".to_string(),
        })
        .unwrap();
        u.add_event(UpdateEvent::AddNodeLabel {
            node_name: "first_node".to_string(),
            anno_ns: DEFAULT_NS.to_string(),
            anno_name: "an_annotation".to_string(),
            anno_value: "something".to_string(),
        })
        .unwrap();

        u.add_event(UpdateEvent::AddEdge {
            source_node: "first_node".to_string(),
            target_node: "second_node".to_string(),
            component_type: "Edge".to_string(),
            layer: "some_ns".to_string(),
            component_name: "test_component".to_string(),
        })
        .unwrap();

        let mut g: Graph<DefaultComponentType> = Graph::new(false).unwrap();
        g.apply_update(&mut u, |_| {}).unwrap();

        // export to GraphML, read generated XML and compare it
        let mut xml_data: Vec<u8> = Vec::default();
        export(&g, Some(TEST_CONFIG), &mut xml_data, |_| {}).unwrap();
        let expected = include_str!("graphml_example.graphml");
        let actual = String::from_utf8(xml_data).unwrap();
        assert_eq!(expected, actual);
    }

    #[test]
    fn export_graphml_sorted() {
        // Create a sample graph using the simple type
        let mut u = GraphUpdate::new();

        u.add_event(UpdateEvent::AddNode {
            node_name: "1".to_string(),
            node_type: "node".to_string(),
        })
        .unwrap();
        u.add_event(UpdateEvent::AddNode {
            node_name: "2".to_string(),
            node_type: "node".to_string(),
        })
        .unwrap();
        u.add_event(UpdateEvent::AddNodeLabel {
            node_name: "1".to_string(),
            anno_ns: DEFAULT_NS.to_string(),
            anno_name: "an_annotation".to_string(),
            anno_value: "something".to_string(),
        })
        .unwrap();

        u.add_event(UpdateEvent::AddEdge {
            source_node: "1".to_string(),
            target_node: "2".to_string(),
            component_type: "Edge".to_string(),
            layer: "some_ns".to_string(),
            component_name: "test_component".to_string(),
        })
        .unwrap();

        let mut g: Graph<DefaultComponentType> = Graph::new(false).unwrap();
        g.apply_update(&mut u, |_| {}).unwrap();

        // export to GraphML, read generated XML and compare it
        let mut xml_data: Vec<u8> = Vec::default();
        export_stable_order(&g, Some(TEST_CONFIG), &mut xml_data, |_| {}).unwrap();
        let expected = include_str!("graphml_example sorted.graphml");
        let actual = String::from_utf8(xml_data).unwrap();
        assert_eq!(expected, actual);
    }

    #[test]
    fn import_graphml() {
        let input_xml = std::io::Cursor::new(
            include_str!("graphml_example.graphml")
                .as_bytes()
                .to_owned(),
        );
        let (g, config_str) = import(input_xml, false, |_| {}).unwrap();

        // Check that all nodes, edges and annotations have been created
        let first_node_id = g
            .node_annos
            .get_node_id_from_name("first_node")
            .unwrap()
            .unwrap();
        let second_node_id = g
            .node_annos
            .get_node_id_from_name("second_node")
            .unwrap()
            .unwrap();

        let first_node_annos = g
            .get_node_annos()
            .get_annotations_for_item(&first_node_id)
            .unwrap();
        assert_eq!(3, first_node_annos.len());
        assert_eq!(
            Some(Cow::Borrowed("something")),
            g.get_node_annos()
                .get_value_for_item(
                    &first_node_id,
                    &AnnoKey {
                        ns: DEFAULT_NS.into(),
                        name: "an_annotation".into(),
                    }
                )
                .unwrap()
        );

        assert_eq!(
            2,
            g.get_node_annos()
                .get_annotations_for_item(&second_node_id)
                .unwrap()
                .len()
        );

        let component = g.get_all_components(Some(DefaultComponentType::Edge), None);
        assert_eq!(1, component.len());
        assert_eq!("some_ns", component[0].layer);
        assert_eq!("test_component", component[0].name);

        let test_gs = g.get_graphstorage_as_ref(&component[0]).unwrap();
        assert_eq!(
            Some(1),
            test_gs.distance(first_node_id, second_node_id).unwrap()
        );

        assert_eq!(Some(TEST_CONFIG), config_str.as_deref());
    }
}
