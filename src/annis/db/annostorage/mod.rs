pub mod inmemory;
pub mod ondisk;
mod symboltable;

use crate::annis::db::{Match, ValueSearch};
use crate::annis::errors::*;
use crate::annis::types::{AnnoKey, Annotation};
use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;

use crate::malloc_size_of::MallocSizeOf;

/// Access annotations for nodes or edges.
pub trait AnnotationStorage<T>: Send + Sync + MallocSizeOf
where
    T: Send + Sync + MallocSizeOf,
{
    /// Insert an annotation `anno` (with annotation key and value) for an item `item`.
    fn insert(&mut self, item: T, anno: Annotation) -> Result<()>;

    /// Get all the annotation keys of a node, filtered by the optional namespace (`ns`) and `name`.
    fn get_all_keys_for_item(
        &self,
        item: &T,
        ns: Option<&str>,
        name: Option<&str>,
    ) -> Vec<Arc<AnnoKey>>;

    /// Remove the annotation given by its `key` for a specific `item`
    /// Returns the value for that annotation, if it existed.
    fn remove_annotation_for_item(&mut self, item: &T, key: &AnnoKey) -> Option<Cow<str>>;

    /// Remove all annotations.
    fn clear(&mut self);

    /// Get all qualified annotation names (including namespace) for a given annotation name
    fn get_qnames(&self, name: &str) -> Vec<AnnoKey>;

    /// Get all annotations for an `item` (node or edge).
    fn get_annotations_for_item(&self, item: &T) -> Vec<Annotation>;

    /// Get the annotation for a given `item` and the annotation `key`.
    fn get_value_for_item(&self, item: &T, key: &AnnoKey) -> Option<Cow<str>>;

    /// Get the matching annotation keys for each item in the iterator.
    ///
    /// This function allows to filter the received annotation keys by the specifying the namespace and name.
    fn get_keys_for_iterator(
        &self,
        ns: Option<&str>,
        name: Option<&str>,
        it: Box<dyn Iterator<Item = T>>,
    ) -> Vec<Match>;

    /// Return the total number of annotations contained in this `AnnotationStorage`.
    fn number_of_annotations(&self) -> usize;

    /// Return the number of annotations contained in this `AnnotationStorage` filtered by `name` and optional namespace (`ns`).
    fn number_of_annotations_by_name(&self, ns: Option<&str>, name: &str) -> usize;

    /// Returns an iterator for all items that exactly match the given annotation constraints.
    /// The annotation `name` must be given as argument, the other arguments are optional.
    ///
    /// - `namespace`- If given, only annotations having this namespace are returned.
    /// - `name`  - Only annotations with this name are returned.
    /// - `value` - Constrain the value of the annotaion.
    ///
    /// The result is an iterator over matches.
    /// A match contains the node ID and the qualifed name of the matched annotation
    /// (e.g. there can be multiple annotations with the same name if the namespace is different).
    fn exact_anno_search<'a>(
        &'a self,
        namespace: Option<&str>,
        name: &str,
        value: ValueSearch<&str>,
    ) -> Box<dyn Iterator<Item = Match> + 'a>;

    /// Returns an iterator for all items where the value matches the regular expression.
    /// The annotation `name` and the `pattern` for the value must be given as argument, the  
    /// `namespace` argument is optional and can be used as additional constraint.
    ///
    /// - `namespace`- If given, only annotations having this namespace are returned.
    /// - `name`  - Only annotations with this name are returned.
    /// - `pattern` - If given, only annotation having a value that mattches this pattern are returned.
    /// - `negated` - If true, find all annotations that do not match the value
    ///
    /// The result is an iterator over matches.
    /// A match contains the node ID and the qualifed name of the matched annotation
    /// (e.g. there can be multiple annotations with the same name if the namespace is different).
    fn regex_anno_search<'a>(
        &'a self,
        namespace: Option<&str>,
        name: &str,
        pattern: &str,
        negated: bool,
    ) -> Box<dyn Iterator<Item = Match> + 'a>;

    /// Estimate the number of results for an [annotation exact search](#tymethod.exact_anno_search) for a given an inclusive value range.
    ///
    /// - `ns` - If given, only annotations having this namespace are considered.
    /// - `name`  - Only annotations with this name are considered.
    /// - `lower_val`- Inclusive lower bound for the annotation value.
    /// - `upper_val`- Inclusive upper bound for the annotation value.
    fn guess_max_count(
        &self,
        ns: Option<&str>,
        name: &str,
        lower_val: &str,
        upper_val: &str,
    ) -> usize;

    /// Estimate the number of results for an [annotation regular expression search](#tymethod.regex_anno_search)
    /// for a given pattern.
    ///
    /// - `ns` - If given, only annotations having this namespace are considered.
    /// - `name`  - Only annotations with this name are considered.
    /// - `pattern`- The regular expression pattern.
    fn guess_max_count_regex(&self, ns: Option<&str>, name: &str, pattern: &str) -> usize;

    /// Estimate the most frequent value for a given annotation `name` with an optional namespace (`ns`).
    ///
    /// If more than one qualified annotation name matches the defnition, the more frequent value is used.
    fn guess_most_frequent_value(&self, ns: Option<&str>, name: &str) -> Option<Cow<str>>;

    /// Return a list of all existing values for a given annotation `key`.
    /// If the `most_frequent_first` parameter is true, the results are sorted by their frequency.
    fn get_all_values(&self, key: &AnnoKey, most_frequent_first: bool) -> Vec<Cow<str>>;

    /// Get all the annotation keys which are part of this annotation storage
    fn annotation_keys(&self) -> Vec<AnnoKey>;

    /// Return the item with the largest item which has an annotation value in this annotation storage.
    ///
    /// This can be used to calculate new IDs for new items.
    fn get_largest_item(&self) -> Option<T>;

    /// (Re-) calculate the internal statistics needed for estimitating annotation values.
    ///
    /// An annotation storage can not have a valid statistics, in which case the estimitation function will not return
    /// valid results.
    fn calculate_statistics(&mut self);

    /// Load the annotation from an external `location`.
    fn load_annotations_from(&mut self, location: &Path) -> Result<()>;

    /// Save the current annotation to a `location` on the disk, but do not remember this location.
    fn save_annotations_to(&self, location: &Path) -> Result<()>;
}
