use std::collections::{BTreeSet, HashSet};

use graphannis_core::errors::GraphAnnisCoreError;
use graphannis_core::graph::{DEFAULT_NS, NODE_NAME_KEY};
use graphannis_core::{
    annostorage::{Match, MatchGroup},
    errors::Result as CoreResult,
    graph::Graph,
    types::{Component, Edge, NodeID},
};
use smallvec::smallvec;

use crate::annis::db::token_helper::TokenHelper;
use crate::annis::errors::GraphAnnisError;
use crate::try_as_option;
use crate::{annis::errors::Result, model::AnnotationComponentType, AnnotationGraph};

struct TokenIterator<'a> {
    n: NodeID,
    end: NodeID,
    covering_nodes: Box<dyn Iterator<Item = NodeID>>,
    token_helper: TokenHelper<'a>,
    include_covering_nodes: bool,
}

impl<'a> TokenIterator<'a> {
    fn calculate_covering_nodes(&mut self) -> Result<()> {
        let mut covering_nodes = HashSet::new();

        // add token  itself
        covering_nodes.insert(self.n);

        let n_is_token = self.token_helper.is_token(self.n)?;
        let coverage_gs = self.token_helper.get_gs_coverage();

        // Find covered nodes in all Coverage graph storages
        for gs_cov in coverage_gs.iter() {
            let covered: Box<dyn Iterator<Item = Result<NodeID>>> = if n_is_token {
                Box::new(std::iter::once(Ok(self.n)))
            } else {
                // all covered token
                Box::new(
                    gs_cov
                        .find_connected(self.n, 1, std::ops::Bound::Included(1))
                        .map(|m| m.map_err(GraphAnnisError::from))
                        .fuse(),
                )
            };

            for t in covered {
                let t = t?;
                // get all nodes that are covering the token (in all coverage components)
                for gs_cov in self.token_helper.get_gs_coverage().iter() {
                    for n in gs_cov.get_ingoing_edges(t) {
                        let n = n?;
                        covering_nodes.insert(n);
                    }
                }
                // also add the token itself
                covering_nodes.insert(t);
            }
        }
        self.covering_nodes = Box::new(covering_nodes.into_iter());
        Ok(())
    }
}

impl<'a> Iterator for TokenIterator<'a> {
    type Item = Result<NodeID>;

    fn next(&mut self) -> Option<Self::Item> {
        // Check if we still need to output some covering nodes for the current node
        if self.include_covering_nodes {
            if let Some(next_covering_node) = self.covering_nodes.next() {
                return Some(Ok(next_covering_node));
            }
        }

        // Get the next token in the chain
        let out: CoreResult<Vec<NodeID>> = self
            .token_helper
            .get_gs_ordering_ref()
            .get_outgoing_edges(self.n)
            .collect();
        match out {
            Ok(out) => {
                if let Some(next_node) = out.into_iter().next() {
                    if next_node == self.end {
                        None
                    } else {
                        self.n = next_node;
                        if self.include_covering_nodes {
                            try_as_option!(self.calculate_covering_nodes());
                        }
                        Some(Ok(next_node))
                    }
                } else {
                    None
                }
            }
            Err(e) => Some(Err(e.into())),
        }
    }
}

fn get_left_token_with_offset(
    graph: &Graph<AnnotationComponentType>,
    token_helper: &TokenHelper,
    token: NodeID,
    ctx_left: usize,
    segmentation: Option<String>,
) -> Result<NodeID> {
    if let Some(segmentation) = segmentation {
        // Get the ordering component for this segmentation
        let component_ordering = Component::new(
            AnnotationComponentType::Ordering,
            DEFAULT_NS.into(),
            segmentation.into(),
        );
        let gs_ordering = graph.get_graphstorage_as_ref(&component_ordering).ok_or(
            GraphAnnisCoreError::MissingComponent(component_ordering.to_string()),
        )?;
        // Get all nodes coverging the token and that are part of
        // the matching ordering component
        let mut covering_segmentation_nodes = BTreeSet::new();
        for gs_cov in token_helper.get_gs_coverage() {
            for n in gs_cov.get_ingoing_edges(token) {
                let n = n?;
                if let Some(first_incoming_edge) = gs_ordering.get_ingoing_edges(n).next() {
                    first_incoming_edge?;
                    covering_segmentation_nodes.insert(n);
                }
            }
        }
        // Get the first matching node of the ordering component and
        // retrieve the left-context segmentation node from it
        let first_segmentation_node = covering_segmentation_nodes
            .into_iter()
            .next()
            .ok_or(GraphAnnisError::NoCoveredTokenForSubgraph)?;
        let left_segmentation_node = gs_ordering
            .find_connected_inverse(
                first_segmentation_node,
                ctx_left,
                std::ops::Bound::Included(ctx_left),
            )
            .next()
            .unwrap_or(Ok(first_segmentation_node))?;
        // Use the left-most token of this node as start of the context
        let result = token_helper
            .left_token_for(left_segmentation_node)?
            .unwrap_or(token);
        Ok(result)
    } else {
        let result = token_helper
            .get_gs_ordering()
            .find_connected_inverse(token, ctx_left, std::ops::Bound::Included(ctx_left))
            .next()
            .unwrap_or(Ok(token))?;
        Ok(result)
    }
}

fn get_right_token_with_offset(
    graph: &Graph<AnnotationComponentType>,
    token_helper: &TokenHelper,
    token: NodeID,
    ctx_right: usize,
    segmentation: Option<String>,
) -> Result<NodeID> {
    if let Some(segmentation) = segmentation {
        // Get the ordering component for this segmentation
        let component_ordering = Component::new(
            AnnotationComponentType::Ordering,
            DEFAULT_NS.into(),
            segmentation.into(),
        );
        let gs_ordering = graph.get_graphstorage_as_ref(&component_ordering).ok_or(
            GraphAnnisCoreError::MissingComponent(component_ordering.to_string()),
        )?;
        // Get all nodes coverging the token and that are part of
        // the matching ordering component
        let mut covering_segmentation_nodes = BTreeSet::new();
        for gs_cov in token_helper.get_gs_coverage() {
            for n in gs_cov.get_ingoing_edges(token) {
                let n = n?;
                if gs_ordering.has_outgoing_edges(n)? {
                    covering_segmentation_nodes.insert(n);
                }
            }
        }
        // Get the first matching node of the ordering component and
        // retrieve the right-context segmentation node from it
        let first_segmentation_node = covering_segmentation_nodes
            .into_iter()
            .next()
            .ok_or(GraphAnnisError::NoCoveredTokenForSubgraph)?;
        let right_segmentation_node = gs_ordering
            .find_connected(
                first_segmentation_node,
                ctx_right,
                std::ops::Bound::Included(ctx_right),
            )
            .next()
            .unwrap_or(Ok(first_segmentation_node))?;
        // Use the right-most token of this node as end of the context
        let result = token_helper
            .right_token_for(right_segmentation_node)?
            .unwrap_or(token);
        Ok(result)
    } else {
        let result = token_helper
            .get_gs_ordering()
            .find_connected(token, ctx_right, std::ops::Bound::Included(ctx_right))
            .next()
            .unwrap_or(Ok(token))?;
        Ok(result)
    }
}

/// Creates a new iterator over all token of the match with the context without gaps.
fn new_token_iterator<'a>(
    graph: &'a Graph<AnnotationComponentType>,
    token_helper: TokenHelper<'a>,
    node_ids: &[NodeID],
    ctx_left: usize,
    ctx_right: usize,
    segmentation: Option<String>,
) -> Result<Box<dyn Iterator<Item = Result<u64>> + 'a>> {
    let left_without_context = token_helper
        .left_token_for_group(node_ids)?
        .ok_or(GraphAnnisError::NoCoveredTokenForSubgraph)?;
    let right_without_context = token_helper
        .right_token_for_group(node_ids)?
        .ok_or(GraphAnnisError::NoCoveredTokenForSubgraph)?;

    // Get the token at the borders of the context
    let start = get_left_token_with_offset(
        graph,
        &token_helper,
        left_without_context,
        ctx_left,
        segmentation.clone(),
    )?;
    let end = get_right_token_with_offset(
        graph,
        &token_helper,
        right_without_context,
        ctx_right,
        segmentation,
    )?;
    // Create an iterator using the ordering edges for the given token range
    let it = TokenIterator {
        n: start,
        end,
        token_helper,
        include_covering_nodes: false,
        covering_nodes: Box::new(std::iter::empty()),
    };
    Ok(Box::new(it))
}

#[derive(Clone)]
struct TokenRegion<'a> {
    start_token: NodeID,
    end_token: NodeID,
    token_helper: TokenHelper<'a>,
}

impl<'a> TokenRegion<'a> {
    fn from_node_with_context(
        graph: &'a Graph<AnnotationComponentType>,
        node_id: NodeID,
        ctx_left: usize,
        ctx_right: usize,
        segmentation: Option<String>,
    ) -> Result<TokenRegion<'a>> {
        let token_helper = TokenHelper::new(graph)?;
        let (left_without_context, right_without_context) =
            token_helper.left_right_token_for(node_id)?;
        let left_without_context =
            left_without_context.ok_or(GraphAnnisError::NoCoveredTokenForSubgraph)?;
        let right_without_context =
            right_without_context.ok_or(GraphAnnisError::NoCoveredTokenForSubgraph)?;

        // Get the token at the borders of the context
        let start_token = get_left_token_with_offset(
            graph,
            &token_helper,
            left_without_context,
            ctx_left,
            segmentation.clone(),
        )?;
        let end_token = get_right_token_with_offset(
            graph,
            &token_helper,
            right_without_context,
            ctx_right,
            segmentation,
        )?;
        Ok(TokenRegion {
            start_token,
            end_token,
            token_helper,
        })
    }

    fn into_token_iterator_with_coverage(self) -> TokenIterator<'a> {
        TokenIterator {
            n: self.start_token,
            end: self.end_token,
            token_helper: self.token_helper,
            include_covering_nodes: true,
            covering_nodes: Box::new(std::iter::empty()),
        }
    }
}

/// Creates an iterator over all overlapped non-token nodes of the match with gaps.
fn new_overlapped_nodes_iterator<'a>(
    graph: &'a Graph<AnnotationComponentType>,
    node_ids: &[NodeID],
    ctx_left: usize,
    ctx_right: usize,
    segmentation: Option<String>,
) -> Result<Box<dyn Iterator<Item = Result<u64>> + 'a>> {
    let mut token_iterators = Vec::default();
    for n in node_ids {
        let token_region = TokenRegion::from_node_with_context(
            graph,
            *n,
            ctx_left,
            ctx_right,
            segmentation.clone(),
        )?;
        token_iterators.push(token_region.into_token_iterator_with_coverage());
    }
    // Chain all iterators ov the vector
    let result = token_iterators.into_iter().flat_map(|it| it);
    Ok(Box::new(result))
}

/// Creates an iterator over all parent nodes of the matched nodes in the
/// corpus graph, including data sources.
fn new_parent_nodes_iterator<'a>(
    graph: &'a Graph<AnnotationComponentType>,
    node_ids: &[NodeID],
    ctx_left: usize,
    ctx_right: usize,
    segmentation: Option<String>,
) -> Result<Box<dyn Iterator<Item = Result<u64>> + 'a>> {
    todo!()
}

pub fn new_subgraph_iterator<'a>(
    graph: &'a Graph<AnnotationComponentType>,
    node_ids: Vec<String>,
    ctx_left: usize,
    ctx_right: usize,
    segmentation: Option<String>,
) -> Result<Box<dyn Iterator<Item = Result<MatchGroup>> + 'a>> {
    let token_helper = TokenHelper::new(graph)?;

    // Get the node IDs for the whole match
    let node_ids: Result<Vec<NodeID>> = node_ids
        .into_iter()
        .map(|node_name| {
            let id = graph.get_node_id_from_name(&node_name)?;
            let id = id.ok_or(GraphAnnisError::NoSuchNodeID(node_name))?;
            Ok(id)
        })
        .collect();
    let node_ids = node_ids?;

    let tokens = new_token_iterator(
        graph,
        token_helper,
        &node_ids,
        ctx_left,
        ctx_right,
        segmentation.clone(),
    )?;
    let overlapped_nodes =
        new_overlapped_nodes_iterator(graph, &node_ids, ctx_left, ctx_right, segmentation.clone())?;
    let parent_nodes =
        new_parent_nodes_iterator(graph, &node_ids, ctx_left, ctx_right, segmentation)?;

    // Chain iterators into a single iterator
    let result = tokens.chain(overlapped_nodes).chain(parent_nodes).map(|n| {
        let n = n?;
        let m: MatchGroup = smallvec![Match {
            node: n,
            anno_key: NODE_NAME_KEY.clone(),
        }];
        Ok(m)
    });
    Ok(Box::new(result))
}

pub fn create_subgraph_for_iterator<I>(
    it: I,
    match_idx: &[usize],
    orig_graph: &Graph<AnnotationComponentType>,
    component_type_filter: Option<AnnotationComponentType>,
) -> Result<AnnotationGraph>
where
    I: Iterator<Item = Result<MatchGroup>>,
{
    // We have to keep our own unique set because the query will return "duplicates" whenever the other parts of the
    // match vector differ.
    let mut match_result: BTreeSet<Match> = BTreeSet::new();

    let mut result = AnnotationGraph::new(false)?;

    // create the subgraph description
    for r in it {
        let r = r?;
        trace!("subgraph query found match {:?}", r);
        for i in match_idx.iter().cloned() {
            if i < r.len() {
                let m: &Match = &r[i];
                if !match_result.contains(m) {
                    match_result.insert(m.clone());
                    trace!("subgraph query extracted node {:?}", m.node);
                    create_subgraph_node(m.node, &mut result, orig_graph)?;
                }
            }
        }
    }

    let components = orig_graph.get_all_components(component_type_filter, None);

    for m in &match_result {
        create_subgraph_edge(m.node, &mut result, orig_graph, &components)?;
    }

    Ok(result)
}

fn create_subgraph_node(
    id: NodeID,
    db: &mut AnnotationGraph,
    orig_db: &AnnotationGraph,
) -> Result<()> {
    // add all node labels with the same node ID
    for a in orig_db.get_node_annos().get_annotations_for_item(&id)? {
        db.get_node_annos_mut().insert(id, a)?;
    }
    Ok(())
}
fn create_subgraph_edge(
    source_id: NodeID,
    db: &mut AnnotationGraph,
    orig_db: &AnnotationGraph,
    components: &[Component<AnnotationComponentType>],
) -> Result<()> {
    // find outgoing edges
    for c in components {
        // don't include index components
        let ctype = c.get_type();
        if !((ctype == AnnotationComponentType::Coverage
            && c.layer == "annis"
            && !c.name.is_empty())
            || ctype == AnnotationComponentType::RightToken
            || ctype == AnnotationComponentType::LeftToken)
        {
            if let Some(orig_gs) = orig_db.get_graphstorage(c) {
                for target in orig_gs.get_outgoing_edges(source_id) {
                    let target = target?;
                    if !db
                        .get_node_annos()
                        .get_all_keys_for_item(&target, None, None)?
                        .is_empty()
                    {
                        let e = Edge {
                            source: source_id,
                            target,
                        };
                        if let Ok(new_gs) = db.get_or_create_writable(c) {
                            new_gs.add_edge(e.clone())?;
                        }

                        for a in orig_gs.get_anno_storage().get_annotations_for_item(&Edge {
                            source: source_id,
                            target,
                        })? {
                            if let Ok(new_gs) = db.get_or_create_writable(c) {
                                new_gs.add_edge_annotation(e.clone(), a)?;
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
