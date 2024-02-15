use itertools::Itertools;
use normpath::PathExt;
use std::{
    collections::{BTreeSet, HashSet},
    convert::TryInto,
    fs::File,
    io::BufReader,
    ops::Bound,
    os::unix::fs::FileExt,
    path::PathBuf,
};
use tempfile::tempfile;

use crate::{
    annostorage::{ondisk::AnnoStorageImpl, AnnotationStorage},
    dfs::CycleSafeDFS,
    errors::Result,
    try_as_boxed_iter,
    types::{Edge, NodeID},
};

use super::{EdgeContainer, GraphStatistic, GraphStorage};
use binary_layout::prelude::*;

pub(crate) const MAX_DEPTH: usize = 15;
pub(crate) const SERIALIZATION_ID: &str = "DiskPathV1_D15";
const ENTRY_SIZE: usize = (MAX_DEPTH * 8) + 1;

binary_layout!(node_path, LittleEndian, {
    length: u8,
    nodes: [u8; MAX_DEPTH*8],
});

/// A [GraphStorage] that stores a single path for each node on disk.
pub struct DiskPathStorage {
    paths: std::fs::File,
    paths_file_size: u64,
    annos: AnnoStorageImpl<Edge>,
    stats: Option<GraphStatistic>,
    location: Option<PathBuf>,
}

fn offset_in_file(n: NodeID) -> u64 {
    n * (ENTRY_SIZE as u64)
}

fn offset_in_path(path_idx: usize) -> usize {
    path_idx * 8
}

impl DiskPathStorage {
    pub fn new() -> Result<DiskPathStorage> {
        let paths = tempfile()?;
        Ok(DiskPathStorage {
            paths,
            paths_file_size: 0,
            location: None,
            annos: AnnoStorageImpl::new(None)?,
            stats: None,
        })
    }

    fn get_outgoing_edge(&self, node: NodeID) -> Result<Option<NodeID>> {
        if node > self.max_node_id()? {
            return Ok(None);
        }
        let mut buffer = [0; ENTRY_SIZE];
        self.paths
            .read_exact_at(&mut buffer, offset_in_file(node))?;
        let view = node_path::View::new(&buffer);
        if view.length().read() == 0 {
            // No outgoing edges
            Ok(None)
        } else {
            // Read the node ID at the first position
            let buffer: [u8; 8] = view.nodes()[offset_in_path(0)..offset_in_path(1)].try_into()?;
            Ok(Some(u64::from_le_bytes(buffer)))
        }
    }

    fn max_node_id(&self) -> Result<u64> {
        let number_of_entries = self.paths_file_size / (ENTRY_SIZE as u64);
        Ok(number_of_entries - 1)
    }

    fn path_for_node(&self, node: NodeID) -> Result<Vec<NodeID>> {
        if node > self.max_node_id()? {
            return Ok(Vec::default());
        }
        let mut buffer = [0; ENTRY_SIZE];
        self.paths
            .read_exact_at(&mut buffer, offset_in_file(node))?;
        let view = node_path::View::new(&buffer);
        let length = view.length().read();
        if length == 0 {
            // No outgoing edges
            Ok(Vec::default())
        } else {
            // Add all path elements
            let mut result = Vec::with_capacity(length as usize);
            for i in 0..length {
                let i = i as usize;
                let element_buffer: [u8; 8] =
                    view.nodes()[offset_in_path(i)..offset_in_path(i + 1)].try_into()?;
                let ancestor_id = u64::from_le_bytes(element_buffer);
                result.push(ancestor_id);
            }

            Ok(result)
        }
    }
}

impl EdgeContainer for DiskPathStorage {
    fn get_outgoing_edges<'a>(
        &'a self,
        node: NodeID,
    ) -> Box<dyn Iterator<Item = Result<NodeID>> + 'a> {
        match self.get_outgoing_edge(node) {
            Ok(Some(n)) => Box::new(std::iter::once(Ok(n))),
            Ok(None) => Box::new(std::iter::empty()),
            Err(e) => Box::new(std::iter::once(Err(e))),
        }
    }

    fn get_ingoing_edges<'a>(
        &'a self,
        node: NodeID,
    ) -> Box<dyn Iterator<Item = Result<NodeID>> + 'a> {
        let max_id = try_as_boxed_iter!(self.max_node_id());
        let mut result = BTreeSet::new();
        for source in 0..=max_id {
            let path = try_as_boxed_iter!(self.path_for_node(source));
            if let Some(target) = path.first() {
                if *target == node {
                    result.insert(source);
                }
            }
        }
        Box::new(result.into_iter().map(Ok))
    }

    fn source_nodes<'a>(&'a self) -> Box<dyn Iterator<Item = Result<NodeID>> + 'a> {
        let max_node_id = try_as_boxed_iter!(self.max_node_id());
        // ignore node entries with empty path in result
        let it = (0..=max_node_id)
            .map(move |n| {
                let mut buffer = [0; ENTRY_SIZE];
                self.paths.read_exact_at(&mut buffer, offset_in_file(n))?;
                let view = node_path::View::new(&buffer);

                let path_length = view.length().read();
                if path_length == 0 {
                    Ok(None)
                } else {
                    Ok(Some(n))
                }
            })
            .filter_map_ok(|n| n);
        Box::new(it)
    }
}

impl GraphStorage for DiskPathStorage {
    fn find_connected<'a>(
        &'a self,
        node: NodeID,
        min_distance: usize,
        max_distance: std::ops::Bound<usize>,
    ) -> Box<dyn Iterator<Item = Result<NodeID>> + 'a> {
        let mut result = Vec::default();
        if min_distance == 0 {
            result.push(Ok(node));
        }

        let path = try_as_boxed_iter!(self.path_for_node(node));
        let start = min_distance.saturating_sub(1);

        let end = match max_distance {
            std::ops::Bound::Included(end) => end + 1,
            std::ops::Bound::Excluded(end) => end,
            std::ops::Bound::Unbounded => path.len(),
        };
        let end = end.min(path.len());
        result.extend(path[start..end].iter().map(|n| Ok(*n)));
        Box::new(result.into_iter())
    }

    fn find_connected_inverse<'a>(
        &'a self,
        node: NodeID,
        min_distance: usize,
        max_distance: std::ops::Bound<usize>,
    ) -> Box<dyn Iterator<Item = Result<NodeID>> + 'a> {
        let mut visited = HashSet::<NodeID>::default();
        let max_distance = match max_distance {
            Bound::Unbounded => usize::MAX,
            Bound::Included(max_distance) => max_distance,
            Bound::Excluded(max_distance) => max_distance - 1,
        };

        let it = CycleSafeDFS::<'a>::new_inverse(self, node, min_distance, max_distance)
            .filter_map_ok(move |x| {
                if visited.insert(x.node) {
                    Some(x.node)
                } else {
                    None
                }
            });
        Box::new(it)
    }

    fn distance(&self, source: NodeID, target: NodeID) -> Result<Option<usize>> {
        let path = self.path_for_node(source)?;
        // Find the target node in the path. The path starts at distance "0".
        let result = path
            .into_iter()
            .position(|n| n == target)
            .map(|idx| idx + 1);
        Ok(result)
    }

    fn is_connected(
        &self,
        source: NodeID,
        target: NodeID,
        min_distance: usize,
        max_distance: std::ops::Bound<usize>,
    ) -> Result<bool> {
        let path = self.path_for_node(source)?;
        // There is a connection when the target node is located in the path (given the min/max constraints)
        let start = min_distance.saturating_sub(1).clamp(0, path.len());
        let end = match max_distance {
            Bound::Unbounded => path.len(),
            Bound::Included(max_distance) => max_distance,
            Bound::Excluded(max_distance) => max_distance.saturating_sub(1),
        };
        let end = end.clamp(0, path.len());
        for p in path.into_iter().take(end).skip(start) {
            if p == target {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn get_anno_storage(&self) -> &dyn crate::annostorage::EdgeAnnotationStorage {
        &self.annos
    }

    fn copy(
        &mut self,
        _node_annos: &dyn crate::annostorage::NodeAnnotationStorage,
        orig: &dyn GraphStorage,
    ) -> Result<()> {
        // Create a new file which is large enough to contain the paths for all nodes.
        let max_node_id = orig
            .source_nodes()
            .fold_ok(0, |acc, node_id| acc.max(node_id))?;
        let file_capacity = (max_node_id + 1) * (ENTRY_SIZE as u64);
        let file = tempfile::tempfile()?;
        if file_capacity > 0 {
            file.set_len(file_capacity)?;
        }

        // Get the paths for all source nodes in the original graph storage
        for source in orig.source_nodes().sorted_by(|a, b| {
            let a = a.as_ref().unwrap_or(&0);
            let b = b.as_ref().unwrap_or(&0);
            a.cmp(b)
        }) {
            let source = source?;

            let mut output_bytes = [0; ENTRY_SIZE];
            let mut path_view = node_path::View::new(&mut output_bytes);
            let dfs = CycleSafeDFS::new(orig.as_edgecontainer(), source, 1, MAX_DEPTH);
            for step in dfs {
                let step = step?;
                let target = step.node;
                // Set the new length
                path_view.length_mut().write(step.distance.try_into()?);
                // The distance starts at 1, but we do not repeat the source
                // node in the path
                let offset = offset_in_path(step.distance - 1);
                // Set the node ID at the given position
                let target_node_id_bytes = target.to_le_bytes();
                path_view.nodes_mut()[offset..(offset + 8)]
                    .copy_from_slice(&target_node_id_bytes[..]);

                // Copy all annotations for this edge
                let e = Edge { source, target };
                for a in orig.get_anno_storage().get_annotations_for_item(&e)? {
                    self.annos.insert(e.clone(), a)?;
                }
            }
            // Save the path at the node offset
            file.write_all_at(&output_bytes, offset_in_file(source))?;
        }
        self.paths = file;
        self.paths_file_size = file_capacity;
        self.stats = orig.get_statistics().cloned();
        self.annos.calculate_statistics()?;
        Ok(())
    }

    fn as_edgecontainer(&self) -> &dyn EdgeContainer {
        self
    }

    fn serialization_id(&self) -> String {
        SERIALIZATION_ID.to_string()
    }

    fn load_from(location: &std::path::Path) -> Result<Self>
    where
        Self: std::marker::Sized,
    {
        // Open the new paths file
        let paths_file = location.join("paths.bin");
        let paths = File::open(paths_file)?;
        let paths_file_size = paths.metadata()?.len();

        // Create annotatio storage
        let annos = AnnoStorageImpl::new(Some(
            location.join(crate::annostorage::ondisk::SUBFOLDER_NAME),
        ))?;

        // Read stats
        let stats_path = location.join("edge_stats.bin");
        let f_stats = std::fs::File::open(stats_path)?;
        let input = std::io::BufReader::new(f_stats);
        let stats = bincode::deserialize_from(input)?;

        Ok(Self {
            paths,
            paths_file_size,
            annos,
            stats,
            location: Some(location.to_path_buf()),
        })
    }

    fn save_to(&self, location: &std::path::Path) -> Result<()> {
        // Make sure the output location exists before trying to normalize the paths
        std::fs::create_dir_all(location)?;
        // Normalize all paths to check if they are the same
        let new_location = location.normalize()?;
        if let Some(old_location) = &self.location {
            let old_location = old_location.normalize()?;
            if new_location == old_location {
                // This is an immutable graph storage so there can't be any
                // changes to write to the existing location we already use.
                return Ok(());
            }
        }
        // Copy the current paths file to the new location
        let new_paths_file = new_location.join("paths.bin");
        let mut new_paths = File::create(new_paths_file)?;
        let mut reader = BufReader::new(&self.paths);
        std::io::copy(&mut reader, &mut new_paths)?;

        self.annos.save_annotations_to(location)?;
        // Write stats with bincode
        let stats_path = location.join("edge_stats.bin");
        let f_stats = std::fs::File::create(stats_path)?;
        let mut writer = std::io::BufWriter::new(f_stats);
        bincode::serialize_into(&mut writer, &self.stats)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        graph::storage::{adjacencylist::AdjacencyListStorage, WriteableGraphStorage},
        types::{AnnoKey, Annotation},
    };
    use pretty_assertions::assert_eq;

    /// Creates an example graph storage with the folllowing structure:
    ///
    /// ```
    /// 0   1   2   3  4    5
    ///  \ /     \ /    \  /
    ///   6       7       8
    ///   |       |       |
    ///   9      10      11
    ///    \      |      /
    ///     \     |     /
    ///      \    |    /
    ///       \   |   /
    ///        \  |  /
    ///           12
    ///   
    /// ```
    fn create_topdown_gs() -> Result<AdjacencyListStorage> {
        let mut orig = AdjacencyListStorage::new();

        // First layer
        orig.add_edge((0, 6).into())?;
        orig.add_edge((1, 6).into())?;
        orig.add_edge((2, 7).into())?;
        orig.add_edge((3, 7).into())?;
        orig.add_edge((4, 8).into())?;
        orig.add_edge((5, 8).into())?;

        // Second layer
        orig.add_edge((6, 9).into())?;
        orig.add_edge((7, 10).into())?;
        orig.add_edge((8, 11).into())?;

        // Third layer
        orig.add_edge((9, 12).into())?;
        orig.add_edge((10, 12).into())?;
        orig.add_edge((11, 12).into())?;

        // Add annotations to last layer
        let key = AnnoKey {
            name: "example".into(),
            ns: "default_ns".into(),
        };
        let anno = Annotation {
            key,
            val: "last".into(),
        };
        orig.add_edge_annotation((9, 12).into(), anno.clone())?;
        orig.add_edge_annotation((10, 12).into(), anno.clone())?;
        orig.add_edge_annotation((11, 12).into(), anno.clone())?;

        Ok(orig)
    }

    #[test]
    fn test_source_nodes() {
        // Create an example graph storage to copy the value from
        let node_annos = AnnoStorageImpl::new(None).unwrap();
        let orig = create_topdown_gs().unwrap();
        let mut target = DiskPathStorage::new().unwrap();
        target.copy(&node_annos, &orig).unwrap();

        let result: Result<Vec<_>> = target.source_nodes().collect();
        let mut result = result.unwrap();
        result.sort();

        assert_eq!(vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11], result);
    }

    #[test]
    fn test_outgoing_edges() {
        // Create an example graph storage to copy the value from
        let node_annos = AnnoStorageImpl::new(None).unwrap();
        let orig = create_topdown_gs().unwrap();
        let mut target = DiskPathStorage::new().unwrap();
        target.copy(&node_annos, &orig).unwrap();

        let result: Result<Vec<_>> = target.get_outgoing_edges(0).collect();
        assert_eq!(vec![6], result.unwrap());

        let result: Result<Vec<_>> = target.get_outgoing_edges(3).collect();
        assert_eq!(vec![7], result.unwrap());

        let result: Result<Vec<_>> = target.get_outgoing_edges(7).collect();
        assert_eq!(vec![10], result.unwrap());

        let result: Result<Vec<_>> = target.get_outgoing_edges(11).collect();
        assert_eq!(vec![12], result.unwrap());

        let result: Result<Vec<_>> = target.get_outgoing_edges(12).collect();
        assert_eq!(0, result.unwrap().len());

        let result: Result<Vec<_>> = target.get_outgoing_edges(100).collect();
        assert_eq!(0, result.unwrap().len());
    }

    #[test]
    fn test_inggoing_edges() {
        // Create an example graph storage to copy the value from
        let node_annos = AnnoStorageImpl::new(None).unwrap();
        let orig = create_topdown_gs().unwrap();
        let mut target = DiskPathStorage::new().unwrap();
        target.copy(&node_annos, &orig).unwrap();

        let result: Result<Vec<_>> = target.get_ingoing_edges(12).collect();
        let mut result = result.unwrap();
        result.sort();
        assert_eq!(vec![9, 10, 11], result);

        let result: Result<Vec<_>> = target.get_ingoing_edges(10).collect();
        let mut result = result.unwrap();
        result.sort();
        assert_eq!(vec![7], result);

        let result: Result<Vec<_>> = target.get_ingoing_edges(8).collect();
        let mut result = result.unwrap();
        result.sort();
        assert_eq!(vec![4, 5], result);

        let result: Result<Vec<_>> = target.get_ingoing_edges(0).collect();
        assert_eq!(0, result.unwrap().len());

        let result: Result<Vec<_>> = target.get_ingoing_edges(1).collect();
        assert_eq!(0, result.unwrap().len());

        let result: Result<Vec<_>> = target.get_ingoing_edges(100).collect();
        assert_eq!(0, result.unwrap().len());
    }

    #[test]
    fn test_path_for_node() {
        // Create an example graph storage to copy the value from
        let node_annos = AnnoStorageImpl::new(None).unwrap();
        let orig = create_topdown_gs().unwrap();
        let mut target = DiskPathStorage::new().unwrap();
        target.copy(&node_annos, &orig).unwrap();

        assert_eq!(vec![6, 9, 12], target.path_for_node(0).unwrap());
        assert_eq!(vec![9, 12], target.path_for_node(6).unwrap());
        assert_eq!(vec![12], target.path_for_node(10).unwrap());

        assert_eq!(vec![7, 10, 12], target.path_for_node(2).unwrap());
        assert_eq!(vec![10, 12], target.path_for_node(7).unwrap());
        assert_eq!(vec![12], target.path_for_node(10).unwrap());

        assert_eq!(0, target.path_for_node(100).unwrap().len());
    }

    #[test]
    fn test_find_connected() {
        // Create an example graph storage to copy the value from
        let node_annos = AnnoStorageImpl::new(None).unwrap();
        let orig = create_topdown_gs().unwrap();
        let mut target = DiskPathStorage::new().unwrap();
        target.copy(&node_annos, &orig).unwrap();

        let result: Result<Vec<_>> = target
            .find_connected(0, 0, std::ops::Bound::Unbounded)
            .collect();
        assert_eq!(vec![0, 6, 9, 12], result.unwrap());

        let result: Result<Vec<_>> = target
            .find_connected(0, 1, std::ops::Bound::Unbounded)
            .collect();
        assert_eq!(vec![6, 9, 12], result.unwrap());

        let result: Result<Vec<_>> = target
            .find_connected(1, 0, std::ops::Bound::Unbounded)
            .collect();
        assert_eq!(vec![1, 6, 9, 12], result.unwrap());

        let result: Result<Vec<_>> = target
            .find_connected(7, 1, std::ops::Bound::Included(2))
            .collect();
        assert_eq!(vec![10, 12], result.unwrap());

        let result: Result<Vec<_>> = target
            .find_connected(7, 1, std::ops::Bound::Excluded(1))
            .collect();
        assert_eq!(vec![10], result.unwrap());

        let result: Result<Vec<_>> = target
            .find_connected(10, 1, std::ops::Bound::Unbounded)
            .collect();
        assert_eq!(vec![12], result.unwrap());
    }

    #[test]
    fn test_find_connected_inverse() {
        let node_annos = AnnoStorageImpl::new(None).unwrap();
        let orig = create_topdown_gs().unwrap();
        let mut target = DiskPathStorage::new().unwrap();
        target.copy(&node_annos, &orig).unwrap();

        let result: Result<Vec<_>> = target
            .find_connected_inverse(12, 0, Bound::Unbounded)
            .collect();
        let mut result = result.unwrap();
        result.sort();
        assert_eq!(vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], result);

        let result: Result<Vec<_>> = target
            .find_connected_inverse(12, 1, Bound::Excluded(2))
            .collect();
        let mut result = result.unwrap();
        result.sort();
        assert_eq!(vec![9, 10, 11], result);

        let result: Result<Vec<_>> = target
            .find_connected_inverse(10, 1, Bound::Included(2))
            .collect();
        let mut result = result.unwrap();
        result.sort();
        assert_eq!(vec![2, 3, 7], result);

        let result: Result<Vec<_>> = target
            .find_connected_inverse(12, 3, Bound::Included(3))
            .collect();
        let mut result = result.unwrap();
        result.sort();
        assert_eq!(vec![0, 1, 2, 3, 4, 5], result);
    }

    #[test]
    fn test_distance() {
        let node_annos = AnnoStorageImpl::new(None).unwrap();
        let orig = create_topdown_gs().unwrap();
        let mut target = DiskPathStorage::new().unwrap();
        target.copy(&node_annos, &orig).unwrap();

        assert_eq!(None, target.distance(7, 7).unwrap());
        assert_eq!(None, target.distance(12, 1).unwrap());
        assert_eq!(Some(1), target.distance(0, 6).unwrap());
        assert_eq!(Some(1), target.distance(3, 7).unwrap());
        assert_eq!(Some(1), target.distance(4, 8).unwrap());
        assert_eq!(Some(2), target.distance(4, 11).unwrap());
        assert_eq!(Some(2), target.distance(6, 12).unwrap());
        assert_eq!(Some(3), target.distance(2, 12).unwrap());
        assert_eq!(Some(3), target.distance(3, 12).unwrap());
    }

    #[test]
    fn test_is_connected() {
        let node_annos = AnnoStorageImpl::new(None).unwrap();
        let orig = create_topdown_gs().unwrap();
        let mut target = DiskPathStorage::new().unwrap();
        target.copy(&node_annos, &orig).unwrap();

        assert_eq!(
            false,
            target.is_connected(7, 7, 0, Bound::Unbounded).unwrap()
        );
        assert_eq!(
            false,
            target.is_connected(12, 1, 0, Bound::Unbounded).unwrap()
        );
        assert_eq!(
            true,
            target.is_connected(0, 6, 1, Bound::Included(1)).unwrap()
        );
        assert_eq!(
            true,
            target.is_connected(3, 7, 1, Bound::Excluded(2)).unwrap()
        );
        assert_eq!(
            true,
            target.is_connected(4, 8, 1, Bound::Unbounded).unwrap()
        );
        assert_eq!(
            true,
            target.is_connected(4, 11, 2, Bound::Excluded(4)).unwrap()
        );
        assert_eq!(
            true,
            target.is_connected(6, 12, 1, Bound::Included(2)).unwrap()
        );
        assert_eq!(
            true,
            target.is_connected(2, 12, 3, Bound::Unbounded).unwrap()
        );
        assert_eq!(
            true,
            target.is_connected(3, 12, 3, Bound::Included(3)).unwrap()
        );
    }

    #[test]
    fn test_save_load() {
        // Create an example graph storage to copy the value from
        let node_annos = AnnoStorageImpl::new(None).unwrap();
        let orig = create_topdown_gs().unwrap();
        let mut save_gs = DiskPathStorage::new().unwrap();
        save_gs.copy(&node_annos, &orig).unwrap();

        let tmp_location = tempfile::TempDir::new().unwrap();
        save_gs.save_to(tmp_location.path()).unwrap();

        let new_gs = DiskPathStorage::load_from(tmp_location.path()).unwrap();

        let result: Result<Vec<_>> = new_gs.source_nodes().collect();
        let mut result = result.unwrap();
        result.sort();

        assert_eq!(vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11], result);

        for source in 9..=11 {
            let edge_anno = new_gs
                .get_anno_storage()
                .get_annotations_for_item(&(source, 12).into())
                .unwrap();
            assert_eq!(1, edge_anno.len());
            assert_eq!("default_ns", edge_anno[0].key.ns);
            assert_eq!("example", edge_anno[0].key.name);
            assert_eq!("last", edge_anno[0].val);
        }
    }
}
