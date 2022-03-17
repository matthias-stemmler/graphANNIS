use super::EdgeContainer;
use crate::{
    errors::{GraphAnnisCoreError, Result},
    types::NodeID,
};
use rustc_hash::FxHashSet;

#[derive(MallocSizeOf)]
pub struct UnionEdgeContainer<'a> {
    containers: Vec<&'a dyn EdgeContainer>,
}

impl<'a> UnionEdgeContainer<'a> {
    pub fn new(containers: Vec<&'a dyn EdgeContainer>) -> UnionEdgeContainer<'a> {
        UnionEdgeContainer { containers }
    }
}

impl<'a> EdgeContainer for UnionEdgeContainer<'a> {
    fn get_outgoing_edges<'b>(
        &'b self,
        node: NodeID,
    ) -> Box<dyn Iterator<Item = Result<NodeID>> + 'b> {
        // Use a hash set so target nodes are only returned once
        let mut targets: FxHashSet<NodeID> = FxHashSet::default();
        // Collect all possible errors when trying to get the outgoing edges
        let mut errors: Vec<GraphAnnisCoreError> = Vec::new();
        for c in self.containers.iter() {
            let outgoing: Result<Vec<NodeID>> = c.get_outgoing_edges(node).collect();
            match outgoing {
                Ok(outgoing) => targets.extend(outgoing),
                Err(e) => errors.push(e),
            }
        }
        if errors.is_empty() {
            Box::from(targets.into_iter().map(|o| Ok(o)))
        } else {
            // Only return the errors
            Box::from(errors.into_iter().map(|e| Err(e)))
        }
    }

    fn get_ingoing_edges<'b>(
        &'b self,
        node: NodeID,
    ) -> Box<dyn Iterator<Item = Result<NodeID>> + 'b> {
        // Use a hash set so target nodes are only returned once
        let mut sources: FxHashSet<NodeID> = FxHashSet::default();
        // Collect all possible errors when trying to get the outgoing edges
        let mut errors: Vec<GraphAnnisCoreError> = Vec::new();
        for c in self.containers.iter() {
            let ingoing: Result<Vec<NodeID>> = c.get_ingoing_edges(node).collect();
            match ingoing {
                Ok(ingoing) => sources.extend(ingoing),
                Err(e) => errors.push(e),
            }
        }
        if errors.is_empty() {
            Box::from(sources.into_iter().map(|o| Ok(o)))
        } else {
            // Only return the errors
            Box::from(errors.into_iter().map(|e| Err(e)))
        }
    }

    fn source_nodes<'b>(&'b self) -> Box<dyn Iterator<Item = NodeID> + 'b> {
        let mut sources = FxHashSet::default();
        for c in self.containers.iter() {
            sources.extend(c.source_nodes());
        }
        Box::from(sources.into_iter())
    }
}
