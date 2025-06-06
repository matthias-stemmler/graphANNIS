use crate::annis::db::aql::disjunction::Disjunction;
use crate::annis::db::aql::Config;
use crate::annis::db::exec::{EmptyResultSet, ExecutionNode, ExecutionNodeDesc};
use crate::annis::errors::*;
use crate::annis::util::TimeoutCheck;
use crate::AnnotationGraph;
use graphannis_core::annostorage::match_group_with_symbol_ids;
use graphannis_core::annostorage::symboltable::SymbolTable;
use graphannis_core::{
    annostorage::MatchGroup,
    types::{AnnoKey, NodeID},
};
use std::collections::HashMap;
use std::fmt::Formatter;
use transient_btree_index::{BtreeConfig, BtreeIndex};

pub struct ExecutionPlan<'a> {
    plans: Vec<Box<dyn ExecutionNode<Item = Result<MatchGroup>> + 'a>>,
    current_plan: usize,
    descriptions: Vec<Option<ExecutionNodeDesc>>,
    inverse_node_pos: Vec<Option<Vec<usize>>>,
    proxy_mode: bool,
    unique_result_set: BtreeIndex<Vec<(NodeID, usize)>, bool>,
    anno_key_symbols: SymbolTable<AnnoKey>,
}

impl<'a> ExecutionPlan<'a> {
    pub fn from_disjunction(
        query: &'a Disjunction,
        db: &'a AnnotationGraph,
        config: &Config,
        timeout: TimeoutCheck,
    ) -> Result<ExecutionPlan<'a>> {
        let mut plans: Vec<Box<dyn ExecutionNode<Item = Result<MatchGroup>> + 'a>> = Vec::new();
        let mut descriptions = Vec::new();
        let mut inverse_node_pos = Vec::new();
        for alt in &query.alternatives {
            let p = alt.make_exec_node(db, config, timeout);
            if let Ok(p) = p {
                descriptions.push(p.get_desc().cloned());

                if let Some(desc) = p.get_desc() {
                    // check if node position mapping is actually needed
                    let node_pos_needed = desc
                        .node_pos
                        .iter()
                        .any(|(target_pos, stream_pos)| target_pos != stream_pos);
                    if node_pos_needed {
                        // invert the node position mapping
                        let new_mapping_map: HashMap<usize, usize> = desc
                            .node_pos
                            .iter()
                            .map(|(target_pos, stream_pos)| (*stream_pos, *target_pos))
                            .collect();
                        let mut new_mapping: Vec<usize> = Vec::with_capacity(new_mapping_map.len());
                        for i in 0..new_mapping_map.len() {
                            let mapping_value = new_mapping_map.get(&i).unwrap_or(&i);
                            new_mapping.push(*mapping_value);
                        }
                        inverse_node_pos.push(Some(new_mapping));
                    } else {
                        inverse_node_pos.push(None);
                    }
                } else {
                    inverse_node_pos.push(None);
                }

                plans.push(p);
            } else if let Err(e) = p {
                if let GraphAnnisError::AQLSemanticError(_) = &e {
                    return Err(e);
                }
            }
        }

        if plans.is_empty() {
            // add a dummy execution step that yields no results
            let no_results_exec = EmptyResultSet {};
            plans.push(Box::new(no_results_exec));
            descriptions.push(None);
        }
        let btree_config = BtreeConfig::default().fixed_value_size(std::mem::size_of::<bool>());
        Ok(ExecutionPlan {
            current_plan: 0,
            descriptions,
            inverse_node_pos,
            proxy_mode: plans.len() == 1,
            plans,
            unique_result_set: BtreeIndex::with_capacity(btree_config, 10_000)?,
            anno_key_symbols: SymbolTable::new(),
        })
    }

    /// Re-orders the match vector from the top execution node to match the
    /// requested query node order. If query nodes are not part of the result,
    /// they are still included in the vector but you can not use the node ID at
    /// this position.
    fn reorder_match(&self, tmp: MatchGroup) -> MatchGroup {
        if let Some(ref inverse_node_pos) = self.inverse_node_pos[self.current_plan] {
            // re-order the matched nodes by the original node position of the query
            let mut result = MatchGroup::new();
            // We cannot assume that every node has a mapping, so use the maximum index
            // in the mapping and not the size of the mapping vector as output vector size.
            let output_size = if let Some(max_item) = inverse_node_pos.iter().max() {
                *max_item + 1
            } else {
                0
            };
            result.resize_with(output_size, Default::default);
            for (stream_pos, m) in tmp.into_iter().enumerate() {
                let target_pos = inverse_node_pos[stream_pos];
                result[target_pos] = m;
            }
            result
        } else {
            tmp
        }
    }

    pub fn estimated_output_size(&self) -> usize {
        let mut estimation = 0;
        for desc in self.descriptions.iter().flatten() {
            if let Some(ref cost) = desc.cost {
                estimation += cost.output;
            }
        }
        estimation
    }

    pub fn is_sorted_by_text(&self) -> bool {
        if self.plans.len() > 1 {
            false
        } else if self.plans.is_empty() {
            true
        } else {
            self.plans[0].is_sorted_by_text()
        }
    }

    fn insert_into_unique_result_set(&mut self, n: &MatchGroup) -> Result<bool> {
        let key = match_group_with_symbol_ids(n, &mut self.anno_key_symbols)?;
        if !self.unique_result_set.contains_key(&key)? {
            self.unique_result_set.insert(key, true)?;
            return Ok(true);
        }
        Ok(false)
    }
}

impl std::fmt::Display for ExecutionPlan<'_> {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        for (i, d) in self.descriptions.iter().enumerate() {
            if i > 0 {
                writeln!(f, "---[OR]---")?;
            }
            if let Some(ref d) = d {
                write!(f, "{}", d.debug_string(""))?;
            } else {
                write!(f, "<no description>")?;
            }
        }
        Ok(())
    }
}

impl Iterator for ExecutionPlan<'_> {
    type Item = Result<MatchGroup>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.proxy_mode {
            // just act as an proxy, but make sure the order is the same as requested in the query
            self.plans[0]
                .next()
                .map(|n| n.map(|n| self.reorder_match(n)))
        } else {
            while self.current_plan < self.plans.len() {
                if let Some(n) = self.plans[self.current_plan].next() {
                    match n {
                        Ok(n) => {
                            let n = self.reorder_match(n);

                            // check if we already outputted this result
                            match self.insert_into_unique_result_set(&n) {
                                Ok(new_result) => {
                                    if new_result {
                                        // new result found, break out of while-loop and return the result
                                        return Some(Ok(n));
                                    }
                                }
                                Err(e) => return Some(Err(e)),
                            }
                        }
                        Err(e) => {
                            return Some(Err(e));
                        }
                    }
                } else {
                    // proceed to next plan
                    self.current_plan += 1;
                }
            }
            None
        }
    }
}
