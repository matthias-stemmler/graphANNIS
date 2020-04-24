use num_traits::{Bounded, FromPrimitive, Num, ToPrimitive};
use std;
use std::fmt;
use std::ops::AddAssign;
use std::string::String;

use std::borrow::Cow;
use std::{convert::TryInto, str::FromStr};
use strum::IntoEnumIterator;
use strum_macros::{EnumIter, EnumString};

use super::serializer::{FixedSizeKeySerializer, KeySerializer};
use crate::graph::{update::UpdateEvent, Graph};
use anyhow::Result;
use malloc_size_of::MallocSizeOf;

/// Unique internal identifier for a single node.
pub type NodeID = u64;

/// The fully qualified name of an annotation.
#[derive(
    Serialize,
    Deserialize,
    Default,
    Eq,
    PartialEq,
    PartialOrd,
    Ord,
    Clone,
    Debug,
    MallocSizeOf,
    Hash,
)]
pub struct AnnoKey {
    /// Name of the annotation.
    pub name: String,
    /// Namespace of the annotation.
    pub ns: String,
}

/// An annotation with a qualified name and a value.
#[derive(Serialize, Deserialize, Default, Eq, PartialEq, PartialOrd, Ord, Clone, Debug, Hash)]
pub struct Annotation {
    /// Qualified name or unique "key" for the annotation
    pub key: AnnoKey,
    /// Value of the annotation
    pub val: String,
}

/// Directed edge between a source and target node which are identified by their ID.
#[derive(
    Serialize,
    Deserialize,
    Eq,
    PartialEq,
    PartialOrd,
    Ord,
    Clone,
    Debug,
    Hash,
    MallocSizeOf,
    Default,
)]
#[repr(C)]
pub struct Edge {
    pub source: NodeID,
    pub target: NodeID,
}

impl Edge {
    pub fn inverse(&self) -> Edge {
        Edge {
            source: self.target,
            target: self.source,
        }
    }
}

impl KeySerializer for Edge {
    fn create_key<'a>(&'a self) -> Cow<'a, [u8]> {
        let mut result = Vec::with_capacity(std::mem::size_of::<NodeID>() * 2);
        result.extend(&self.source.to_be_bytes());
        result.extend(&self.target.to_be_bytes());
        Cow::Owned(result)
    }

    fn parse_key(key: &[u8]) -> Self {
        let id_size = std::mem::size_of::<NodeID>();

        let source = NodeID::from_be_bytes(
            key[..id_size]
                .try_into()
                .expect("Edge deserialization key was too small"),
        );
        let target = NodeID::from_be_bytes(
            key[id_size..]
                .try_into()
                .expect("Edge deserialization key has wrong size"),
        );
        Edge { source, target }
    }
}

impl FixedSizeKeySerializer for Edge {
    fn key_size() -> usize {
        std::mem::size_of::<NodeID>() * 2
    }
}

pub trait ComponentType: Into<u16> + From<u16> + FromStr + ToString + Send + Sync + Clone {
    type UpdateGraphIndex;
    fn init_graph_update_index(_graph: &Graph<Self>) -> Result<Self::UpdateGraphIndex>;

    fn before_update_event(
        _update: &UpdateEvent,
        _graph: &Graph<Self>,
        _index: &mut Self::UpdateGraphIndex,
    ) -> Result<()> {
        Ok(())
    }
    fn after_update_event(
        _update: UpdateEvent,
        _graph: &Graph<Self>,
        _index: &mut Self::UpdateGraphIndex,
    ) -> Result<()> {
        Ok(())
    }
    fn apply_update_graph_index(
        _index: Self::UpdateGraphIndex,
        _graph: &mut Graph<Self>,
    ) -> Result<()> {
        Ok(())
    }

    fn all_component_types() -> Vec<Self>;

    fn default_components() -> Vec<Component> {
        Vec::default()
    }
}

/// A simplified implementation of a `ComponentType` that only has one type of edges.
#[derive(Clone, EnumString, EnumIter, Debug)]
pub enum DefaultComponentType {
    Edge,
}

impl Into<u16> for DefaultComponentType {
    fn into(self) -> u16 {
        0
    }
}

impl From<u16> for DefaultComponentType {
    fn from(_: u16) -> Self {
        DefaultComponentType::Edge
    }
}

impl fmt::Display for DefaultComponentType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

pub struct DefaultGraphIndex;

impl ComponentType for DefaultComponentType {
    type UpdateGraphIndex = DefaultGraphIndex;
    fn init_graph_update_index(_graph: &Graph<Self>) -> Result<Self::UpdateGraphIndex> {
        Ok(DefaultGraphIndex {})
    }
    fn all_component_types() -> Vec<Self> {
        DefaultComponentType::iter().collect()
    }
}

/// Identifies an edge component of the graph.
#[derive(
    Serialize, Deserialize, Eq, PartialEq, PartialOrd, Ord, Hash, Clone, Debug, MallocSizeOf,
)]
pub struct Component {
    /// Type of the component
    pub ctype: u16,
    /// Name of the component
    pub name: String,
    /// A layer name which allows to group different components into the same layer. Can be empty.
    pub layer: String,
}

impl std::fmt::Display for Component {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{:?}/{}/{}", self.ctype, self.layer, self.name)
    }
}

pub trait NumValue:
    Send + Sync + Ord + Num + AddAssign + Clone + Bounded + FromPrimitive + ToPrimitive + MallocSizeOf
{
}

impl NumValue for u64 {}
impl NumValue for u32 {}
impl NumValue for u16 {}
impl NumValue for u8 {}
