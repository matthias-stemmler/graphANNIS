use super::*;
use crate::annis::db::annostorage::ondisk::AnnoStorageImpl;
use crate::annis::db::AnnotationStorage;
use crate::annis::dfs::CycleSafeDFS;
use crate::annis::errors::*;
use crate::annis::util::memory_estimation;
use crate::annis::types::Edge;

use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::BTreeSet;
use std::ops::Bound;
use std::path::{Path, PathBuf};

#[derive(MallocSizeOf)]
pub struct DiskAdjacencyListStorage {
    #[ignore_malloc_size_of = "is stored on disk"]
    db: rocksdb::DB,
    #[with_malloc_size_of_func = "memory_estimation::size_of_pathbuf"]
    location: PathBuf,
    /// A handle to a temporary directory. This must be part of the struct because the temporary directory will
    /// be deleted when this handle is dropped.
    #[with_malloc_size_of_func = "memory_estimation::size_of_option_tempdir"]
    temp_dir: Option<tempfile::TempDir>,

    edges: FxHashMap<NodeID, Vec<NodeID>>,
    inverse_edges: FxHashMap<NodeID, Vec<NodeID>>,
    annos: AnnoStorageImpl<Edge>,
    stats: Option<GraphStatistic>,
}

fn get_fan_outs(edges: &FxHashMap<NodeID, Vec<NodeID>>) -> Vec<usize> {
    let mut fan_outs: Vec<usize> = Vec::new();
    if !edges.is_empty() {
        for outgoing in edges.values() {
            fan_outs.push(outgoing.len());
        }
    }
    // order the fan-outs
    fan_outs.sort();

    fan_outs
}

fn open_db(path: &Path) -> Result<rocksdb::DB> {
    let mut db_opts = rocksdb::Options::default();
    db_opts.create_missing_column_families(true);
    db_opts.create_if_missing(true);
    let mut block_opts = rocksdb::BlockBasedOptions::default();
    block_opts.set_index_type(rocksdb::BlockBasedIndexType::HashSearch);
    block_opts.set_bloom_filter(std::mem::size_of::<NodeID>() as i32, false);
    db_opts.set_block_based_table_factory(&block_opts);

    // use prefixes for the different column families
    let mut opts_edges = rocksdb::Options::default();
    opts_edges.set_prefix_extractor(rocksdb::SliceTransform::create_fixed_prefix(
        std::mem::size_of::<NodeID>(),
    ));
    let cf_edges = rocksdb::ColumnFamilyDescriptor::new("edges", opts_edges);

    let mut opts_inverse_edges = rocksdb::Options::default();
    opts_inverse_edges.set_prefix_extractor(rocksdb::SliceTransform::create_fixed_prefix(
        std::mem::size_of::<NodeID>(),
    ));
    let cf_inverse_edges =
        rocksdb::ColumnFamilyDescriptor::new("inverse_edges", opts_inverse_edges);

    let db = rocksdb::DB::open_cf_descriptors(&db_opts, path, vec![cf_edges, cf_inverse_edges])?;

    Ok(db)
}

impl DiskAdjacencyListStorage {
    pub fn new(location: Option<&Path>) -> Result<DiskAdjacencyListStorage> {
        if let Some(location) = location {
            let db = open_db(location)?;
            let gs = DiskAdjacencyListStorage {
                edges: FxHashMap::default(),
                inverse_edges: FxHashMap::default(),
                annos: AnnoStorageImpl::new(None)?,
                stats: None,
                location: location.to_path_buf(),
                temp_dir: None,
                db,
            };
            Ok(gs)
        } else {
            let tmp_dir = tempfile::Builder::new()
                .prefix("graphannis-ondisk-adjacency-")
                .tempdir()?;
            let db = open_db(tmp_dir.as_ref())?;
            let gs = DiskAdjacencyListStorage {
                edges: FxHashMap::default(),
                inverse_edges: FxHashMap::default(),
                annos: AnnoStorageImpl::new(None)?,
                stats: None,
                location: tmp_dir.as_ref().to_path_buf(),
                temp_dir: Some(tmp_dir),
                db: db,
            };
            Ok(gs)
        }
    }

    pub fn clear(&mut self) -> Result<()> {
        self.edges.clear();
        self.inverse_edges.clear();
        self.annos.clear()?;
        self.stats = None;
        Ok(())
    }
}

impl EdgeContainer for DiskAdjacencyListStorage {
    fn get_outgoing_edges<'a>(&'a self, node: NodeID) -> Box<dyn Iterator<Item = NodeID> + 'a> {
        if let Some(outgoing) = self.edges.get(&node) {
            return match outgoing.len() {
                0 => Box::new(std::iter::empty()),
                1 => Box::new(std::iter::once(outgoing[0])),
                _ => Box::new(outgoing.iter().cloned()),
            };
        }
        Box::new(std::iter::empty())
    }

    fn get_ingoing_edges<'a>(&'a self, node: NodeID) -> Box<dyn Iterator<Item = NodeID> + 'a> {
        if let Some(ingoing) = self.inverse_edges.get(&node) {
            return match ingoing.len() {
                0 => Box::new(std::iter::empty()),
                1 => Box::new(std::iter::once(ingoing[0])),
                _ => Box::new(ingoing.iter().cloned()),
            };
        }
        Box::new(std::iter::empty())
    }
    fn source_nodes<'a>(&'a self) -> Box<dyn Iterator<Item = NodeID> + 'a> {
        let it = self
            .edges
            .iter()
            .filter(|(_, outgoing)| !outgoing.is_empty())
            .map(|(key, _)| *key);
        Box::new(it)
    }

    fn get_statistics(&self) -> Option<&GraphStatistic> {
        self.stats.as_ref()
    }
}

impl GraphStorage for DiskAdjacencyListStorage {
    fn get_anno_storage(&self) -> &dyn AnnotationStorage<Edge> {
        &self.annos
    }

    fn serialization_id(&self) -> String {
        "AdjacencyListV1".to_owned()
    }

    fn serialize_gs(&self, writer: &mut dyn std::io::Write) -> Result<()> {
        unimplemented!()
    }

    fn deserialize_gs(input: &mut dyn std::io::Read) -> Result<Self>
    where
        for<'de> Self: std::marker::Sized + Deserialize<'de>,
    {
        unimplemented!()
    }

    fn find_connected<'a>(
        &'a self,
        node: NodeID,
        min_distance: usize,
        max_distance: Bound<usize>,
    ) -> Box<dyn Iterator<Item = NodeID> + 'a> {
        let mut visited = FxHashSet::<NodeID>::default();
        let max_distance = match max_distance {
            Bound::Unbounded => usize::max_value(),
            Bound::Included(max_distance) => max_distance,
            Bound::Excluded(max_distance) => max_distance + 1,
        };
        let it = CycleSafeDFS::<'a>::new(self, node, min_distance, max_distance)
            .map(|x| x.node)
            .filter(move |n| visited.insert(n.clone()));
        Box::new(it)
    }

    fn find_connected_inverse<'a>(
        &'a self,
        node: NodeID,
        min_distance: usize,
        max_distance: Bound<usize>,
    ) -> Box<dyn Iterator<Item = NodeID> + 'a> {
        let mut visited = FxHashSet::<NodeID>::default();
        let max_distance = match max_distance {
            Bound::Unbounded => usize::max_value(),
            Bound::Included(max_distance) => max_distance,
            Bound::Excluded(max_distance) => max_distance + 1,
        };

        let it = CycleSafeDFS::<'a>::new_inverse(self, node, min_distance, max_distance)
            .map(|x| x.node)
            .filter(move |n| visited.insert(n.clone()));
        Box::new(it)
    }

    fn distance(&self, source: NodeID, target: NodeID) -> Option<usize> {
        let mut it = CycleSafeDFS::new(self, source, usize::min_value(), usize::max_value())
            .filter(|x| target == x.node)
            .map(|x| x.distance);

        it.next()
    }
    fn is_connected(
        &self,
        source: NodeID,
        target: NodeID,
        min_distance: usize,
        max_distance: std::ops::Bound<usize>,
    ) -> bool {
        let max_distance = match max_distance {
            Bound::Unbounded => usize::max_value(),
            Bound::Included(max_distance) => max_distance,
            Bound::Excluded(max_distance) => max_distance + 1,
        };
        let mut it = CycleSafeDFS::new(self, source, min_distance, max_distance)
            .filter(|x| target == x.node);

        it.next().is_some()
    }

    fn copy(&mut self, _db: &Graph, orig: &dyn GraphStorage) -> Result<()> {
        self.clear()?;

        for source in orig.source_nodes() {
            for target in orig.get_outgoing_edges(source) {
                let e = Edge { source, target };
                self.add_edge(e.clone());
                for a in orig.get_anno_storage().get_annotations_for_item(&e) {
                    self.add_edge_annotation(e.clone(), a)?;
                }
            }
        }

        self.stats = orig.get_statistics().cloned();
        self.annos.calculate_statistics();
        Ok(())
    }

    fn as_writeable(&mut self) -> Option<&mut dyn WriteableGraphStorage> {
        Some(self)
    }
    fn as_edgecontainer(&self) -> &dyn EdgeContainer {
        self
    }

    fn inverse_has_same_cost(&self) -> bool {
        true
    }
}

impl WriteableGraphStorage for DiskAdjacencyListStorage {
    fn add_edge(&mut self, edge: Edge) {
        if edge.source != edge.target {
            // insert to both regular and inverse maps

            let inverse_entry = self
                .inverse_edges
                .entry(edge.target)
                .or_insert_with(Vec::default);
            // no need to insert it: edge already exists
            if let Err(insertion_idx) = inverse_entry.binary_search(&edge.source) {
                inverse_entry.insert(insertion_idx, edge.source);
            }

            let regular_entry = self.edges.entry(edge.source).or_insert_with(Vec::default);
            if let Err(insertion_idx) = regular_entry.binary_search(&edge.target) {
                regular_entry.insert(insertion_idx, edge.target);
            }
            self.stats = None;
            // TODO: invalid graph statistics
        }
    }
    fn add_edge_annotation(&mut self, edge: Edge, anno: Annotation) -> Result<()> {
        if let Some(outgoing) = self.edges.get(&edge.source) {
            if outgoing.contains(&edge.target) {
                self.annos.insert(edge, anno)?;
            }
        }
        Ok(())
    }

    fn delete_edge(&mut self, edge: &Edge) -> Result<()> {
        if let Some(outgoing) = self.edges.get_mut(&edge.source) {
            if let Ok(idx) = outgoing.binary_search(&edge.target) {
                outgoing.remove(idx);
            }
        }

        if let Some(ingoing) = self.inverse_edges.get_mut(&edge.target) {
            if let Ok(idx) = ingoing.binary_search(&edge.source) {
                ingoing.remove(idx);
            }
        }
        let annos = self.annos.get_annotations_for_item(edge);
        for a in annos {
            self.annos.remove_annotation_for_item(edge, &a.key)?;
        }

        Ok(())
    }
    fn delete_edge_annotation(&mut self, edge: &Edge, anno_key: &AnnoKey) -> Result<()> {
        self.annos.remove_annotation_for_item(edge, anno_key)?;
        Ok(())
    }
    fn delete_node(&mut self, node: NodeID) -> Result<()> {
        // find all both ingoing and outgoing edges
        let mut to_delete = std::collections::LinkedList::<Edge>::new();

        if let Some(outgoing) = self.edges.get(&node) {
            for target in outgoing.iter() {
                to_delete.push_back(Edge {
                    source: node,
                    target: *target,
                })
            }
        }
        if let Some(ingoing) = self.inverse_edges.get(&node) {
            for source in ingoing.iter() {
                to_delete.push_back(Edge {
                    source: *source,
                    target: node,
                })
            }
        }

        for e in to_delete {
            self.delete_edge(&e)?;
        }

        Ok(())
    }

    fn calculate_statistics(&mut self) {
        let mut stats = GraphStatistic {
            max_depth: 1,
            max_fan_out: 0,
            avg_fan_out: 0.0,
            fan_out_99_percentile: 0,
            inverse_fan_out_99_percentile: 0,
            cyclic: false,
            rooted_tree: true,
            nodes: 0,
            dfs_visit_ratio: 0.0,
        };

        self.annos.calculate_statistics();

        let mut has_incoming_edge: BTreeSet<NodeID> = BTreeSet::new();

        // find all root nodes
        let mut roots: BTreeSet<NodeID> = BTreeSet::new();
        {
            let mut all_nodes: BTreeSet<NodeID> = BTreeSet::new();
            for (source, outgoing) in &self.edges {
                roots.insert(*source);
                all_nodes.insert(*source);
                for target in outgoing {
                    all_nodes.insert(*target);

                    if stats.rooted_tree {
                        if has_incoming_edge.contains(target) {
                            stats.rooted_tree = false;
                        } else {
                            has_incoming_edge.insert(*target);
                        }
                    }
                }
            }
            stats.nodes = all_nodes.len();
        }

        if !self.edges.is_empty() {
            for outgoing in self.edges.values() {
                for target in outgoing {
                    roots.remove(&target);
                }
            }
        }

        let fan_outs = get_fan_outs(&self.edges);
        let sum_fan_out: usize = fan_outs.iter().sum();

        if let Some(last) = fan_outs.last() {
            stats.max_fan_out = *last;
        }
        let inverse_fan_outs = get_fan_outs(&self.inverse_edges);

        // get the percentile value(s)
        // set some default values in case there are not enough elements in the component
        if !fan_outs.is_empty() {
            stats.fan_out_99_percentile = fan_outs[fan_outs.len() - 1];
        }
        if !inverse_fan_outs.is_empty() {
            stats.inverse_fan_out_99_percentile = inverse_fan_outs[inverse_fan_outs.len() - 1];
        }
        // calculate the more accurate values
        if fan_outs.len() >= 100 {
            let idx: usize = fan_outs.len() / 100;
            if idx < fan_outs.len() {
                stats.fan_out_99_percentile = fan_outs[idx];
            }
        }
        if inverse_fan_outs.len() >= 100 {
            let idx: usize = inverse_fan_outs.len() / 100;
            if idx < inverse_fan_outs.len() {
                stats.inverse_fan_out_99_percentile = inverse_fan_outs[idx];
            }
        }

        let mut number_of_visits = 0;
        if roots.is_empty() && !self.edges.is_empty() {
            // if we have edges but no roots at all there must be a cycle
            stats.cyclic = true;
        } else {
            for root_node in &roots {
                let mut dfs = CycleSafeDFS::new(self, *root_node, 0, usize::max_value());
                while let Some(step) = dfs.next() {
                    number_of_visits += 1;
                    stats.max_depth = std::cmp::max(stats.max_depth, step.distance);
                }
                if dfs.is_cyclic() {
                    stats.cyclic = true;
                }
            }
        }

        if stats.cyclic {
            stats.rooted_tree = false;
            // it's infinite
            stats.max_depth = 0;
            stats.dfs_visit_ratio = 0.0;
        } else if stats.nodes > 0 {
            stats.dfs_visit_ratio = f64::from(number_of_visits) / (stats.nodes as f64);
        }

        if sum_fan_out > 0 && stats.nodes > 0 {
            stats.avg_fan_out = (sum_fan_out as f64) / (stats.nodes as f64);
        }

        self.stats = Some(stats);
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    use itertools::Itertools;

    #[test]
    fn multiple_paths_find_range() {
        /*
        +---+
        | 1 | -+
        +---+  |
            |    |
            |    |
            v    |
        +---+  |
        | 2 |  |
        +---+  |
            |    |
            |    |
            v    |
        +---+  |
        | 3 | <+
        +---+
            |
            |
            v
        +---+
        | 4 |
        +---+
            |
            |
            v
        +---+
        | 5 |
        +---+
        */

        let mut gs = DiskAdjacencyListStorage::new(None).unwrap();
        gs.add_edge(Edge {
            source: 1,
            target: 2,
        });
        gs.add_edge(Edge {
            source: 2,
            target: 3,
        });
        gs.add_edge(Edge {
            source: 3,
            target: 4,
        });
        gs.add_edge(Edge {
            source: 1,
            target: 3,
        });
        gs.add_edge(Edge {
            source: 4,
            target: 5,
        });

        let mut found: Vec<NodeID> = gs
            .find_connected(1, 3, std::ops::Bound::Included(3))
            .collect();

        assert_eq!(2, found.len());
        found.sort();

        assert_eq!(4, found[0]);
        assert_eq!(5, found[1]);
    }

    #[test]
    fn simple_dag_find_all() {
        /*
        +---+     +---+     +---+     +---+
        | 7 | <-- | 5 | <-- | 3 | <-- | 1 |
        +---+     +---+     +---+     +---+
                    |         |         |
                    |         |         |
                    v         |         v
                  +---+       |       +---+
                  | 6 |       |       | 2 |
                  +---+       |       +---+
                              |         |
                              |         |
                              |         v
                              |       +---+
                              +-----> | 4 |
                                      +---+
        */
        let mut gs = DiskAdjacencyListStorage::new(None).unwrap();

        gs.add_edge(Edge {
            source: 1,
            target: 2,
        });
        gs.add_edge(Edge {
            source: 2,
            target: 4,
        });
        gs.add_edge(Edge {
            source: 1,
            target: 3,
        });
        gs.add_edge(Edge {
            source: 3,
            target: 5,
        });
        gs.add_edge(Edge {
            source: 5,
            target: 7,
        });
        gs.add_edge(Edge {
            source: 5,
            target: 6,
        });
        gs.add_edge(Edge {
            source: 3,
            target: 4,
        });

        assert_eq!(
            vec![2, 3],
            gs.get_outgoing_edges(1).sorted().collect::<Vec<NodeID>>()
        );
        assert_eq!(
            vec![4, 5],
            gs.get_outgoing_edges(3).sorted().collect::<Vec<NodeID>>()
        );
        assert_eq!(0, gs.get_outgoing_edges(6).count());
        assert_eq!(vec![4], gs.get_outgoing_edges(2).collect::<Vec<NodeID>>());

        let mut reachable: Vec<NodeID> = gs.find_connected(1, 1, Bound::Included(100)).collect();
        reachable.sort();
        assert_eq!(vec![2, 3, 4, 5, 6, 7], reachable);

        let mut reachable: Vec<NodeID> = gs.find_connected(3, 2, Bound::Included(100)).collect();
        reachable.sort();
        assert_eq!(vec![6, 7], reachable);

        let mut reachable: Vec<NodeID> = gs.find_connected(1, 2, Bound::Included(4)).collect();
        reachable.sort();
        assert_eq!(vec![4, 5, 6, 7], reachable);

        let reachable: Vec<NodeID> = gs.find_connected(7, 1, Bound::Included(100)).collect();
        assert_eq!(true, reachable.is_empty());
    }

    #[test]
    fn indirect_cycle_statistics() {
        let mut gs = DiskAdjacencyListStorage::new(None).unwrap();

        gs.add_edge(Edge {
            source: 1,
            target: 2,
        });

        gs.add_edge(Edge {
            source: 2,
            target: 3,
        });

        gs.add_edge(Edge {
            source: 3,
            target: 4,
        });

        gs.add_edge(Edge {
            source: 4,
            target: 5,
        });

        gs.add_edge(Edge {
            source: 5,
            target: 2,
        });

        gs.calculate_statistics();
        assert_eq!(true, gs.get_statistics().is_some());
        let stats = gs.get_statistics().unwrap();
        assert_eq!(true, stats.cyclic);
    }

    #[test]
    fn multi_branch_cycle_statistics() {
        let mut gs = DiskAdjacencyListStorage::new(None).unwrap();

        gs.add_edge(Edge {
            source: 903,
            target: 1343,
        });
        gs.add_edge(Edge {
            source: 904,
            target: 1343,
        });
        gs.add_edge(Edge {
            source: 1174,
            target: 1343,
        });
        gs.add_edge(Edge {
            source: 1295,
            target: 1343,
        });
        gs.add_edge(Edge {
            source: 1310,
            target: 1343,
        });
        gs.add_edge(Edge {
            source: 1334,
            target: 1343,
        });
        gs.add_edge(Edge {
            source: 1335,
            target: 1343,
        });
        gs.add_edge(Edge {
            source: 1336,
            target: 1343,
        });
        gs.add_edge(Edge {
            source: 1337,
            target: 1343,
        });
        gs.add_edge(Edge {
            source: 1338,
            target: 1343,
        });
        gs.add_edge(Edge {
            source: 1339,
            target: 1343,
        });
        gs.add_edge(Edge {
            source: 1340,
            target: 1343,
        });
        gs.add_edge(Edge {
            source: 1341,
            target: 1343,
        });
        gs.add_edge(Edge {
            source: 1342,
            target: 1343,
        });
        gs.add_edge(Edge {
            source: 1343,
            target: 1343,
        });
        gs.add_edge(Edge {
            source: 903,
            target: 1342,
        });
        gs.add_edge(Edge {
            source: 904,
            target: 1342,
        });
        gs.add_edge(Edge {
            source: 1174,
            target: 1342,
        });
        gs.add_edge(Edge {
            source: 1295,
            target: 1342,
        });
        gs.add_edge(Edge {
            source: 1310,
            target: 1342,
        });
        gs.add_edge(Edge {
            source: 1334,
            target: 1342,
        });
        gs.add_edge(Edge {
            source: 1335,
            target: 1342,
        });
        gs.add_edge(Edge {
            source: 1336,
            target: 1342,
        });
        gs.add_edge(Edge {
            source: 1337,
            target: 1342,
        });
        gs.add_edge(Edge {
            source: 1338,
            target: 1342,
        });
        gs.add_edge(Edge {
            source: 1339,
            target: 1342,
        });
        gs.add_edge(Edge {
            source: 1340,
            target: 1342,
        });
        gs.add_edge(Edge {
            source: 1341,
            target: 1342,
        });
        gs.add_edge(Edge {
            source: 1342,
            target: 1342,
        });
        gs.add_edge(Edge {
            source: 1343,
            target: 1342,
        });
        gs.add_edge(Edge {
            source: 903,
            target: 1339,
        });
        gs.add_edge(Edge {
            source: 904,
            target: 1339,
        });
        gs.add_edge(Edge {
            source: 1174,
            target: 1339,
        });
        gs.add_edge(Edge {
            source: 1295,
            target: 1339,
        });
        gs.add_edge(Edge {
            source: 1310,
            target: 1339,
        });
        gs.add_edge(Edge {
            source: 1334,
            target: 1339,
        });
        gs.add_edge(Edge {
            source: 1335,
            target: 1339,
        });
        gs.add_edge(Edge {
            source: 1336,
            target: 1339,
        });
        gs.add_edge(Edge {
            source: 1337,
            target: 1339,
        });
        gs.add_edge(Edge {
            source: 1338,
            target: 1339,
        });
        gs.add_edge(Edge {
            source: 1339,
            target: 1339,
        });
        gs.add_edge(Edge {
            source: 1340,
            target: 1339,
        });
        gs.add_edge(Edge {
            source: 1341,
            target: 1339,
        });
        gs.add_edge(Edge {
            source: 1342,
            target: 1339,
        });
        gs.add_edge(Edge {
            source: 1343,
            target: 1339,
        });

        gs.calculate_statistics();
        assert_eq!(true, gs.get_statistics().is_some());
        let stats = gs.get_statistics().unwrap();
        assert_eq!(true, stats.cyclic);
    }
}
