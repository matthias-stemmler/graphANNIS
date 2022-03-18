use crate::annis::db::token_helper;
use crate::annis::db::{aql::model::AnnotationComponentType, token_helper::TokenHelper};
use crate::annis::errors::GraphAnnisError;
use crate::annis::operator::{BinaryOperator, BinaryOperatorSpec};
use crate::annis::operator::{BinaryOperatorBase, BinaryOperatorIndex};
use crate::{annis::operator::EstimationType, errors::Result, graph::Match};
use crate::{try_as_boxed_iter, AnnotationGraph};
use graphannis_core::{graph::DEFAULT_ANNO_KEY, types::Component};
use itertools::Itertools;
use std::any::Any;
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Clone, Debug, PartialOrd, Ord, Hash, PartialEq, Eq)]
pub struct LeftAlignmentSpec;

#[derive(Clone)]
pub struct LeftAlignment<'a> {
    tok_helper: TokenHelper<'a>,
}

impl BinaryOperatorSpec for LeftAlignmentSpec {
    fn necessary_components(
        &self,
        db: &AnnotationGraph,
    ) -> HashSet<Component<AnnotationComponentType>> {
        let mut v = HashSet::default();
        v.extend(token_helper::necessary_components(db));
        v
    }

    fn create_operator<'a>(&self, db: &'a AnnotationGraph) -> Option<BinaryOperator<'a>> {
        let optional_op = LeftAlignment::new(db);
        optional_op.map(|op| BinaryOperator::Index(Box::new(op)))
    }

    fn into_any(self: Arc<Self>) -> Arc<dyn Any> {
        self
    }

    fn any_ref(&self) -> &dyn Any {
        self
    }
}

impl<'a> LeftAlignment<'a> {
    pub fn new(graph: &'a AnnotationGraph) -> Option<LeftAlignment<'a>> {
        let tok_helper = TokenHelper::new(graph)?;

        Some(LeftAlignment { tok_helper })
    }
}

impl<'a> std::fmt::Display for LeftAlignment<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "_l_")
    }
}

impl<'a> BinaryOperatorBase for LeftAlignment<'a> {
    fn filter_match(&self, lhs: &Match, rhs: &Match) -> Result<bool> {
        if let (Some(lhs_token), Some(rhs_token)) = (
            self.tok_helper.left_token_for(lhs.node)?,
            self.tok_helper.left_token_for(rhs.node)?,
        ) {
            Ok(lhs_token == rhs_token)
        } else {
            Ok(false)
        }
    }

    fn is_reflexive(&self) -> bool {
        false
    }

    fn get_inverse_operator<'b>(&self, graph: &'b AnnotationGraph) -> Option<BinaryOperator<'b>> {
        let tok_helper = TokenHelper::new(graph)?;

        Some(BinaryOperator::Index(Box::new(LeftAlignment {
            tok_helper,
        })))
    }

    fn estimation_type(&self) -> Result<EstimationType> {
        if let Some(stats_left) = self.tok_helper.get_gs_left_token().get_statistics() {
            let aligned_nodes_per_token: f64 = stats_left.inverse_fan_out_99_percentile as f64;
            return Ok(EstimationType::Selectivity(
                aligned_nodes_per_token / (stats_left.nodes as f64),
            ));
        }

        Ok(EstimationType::Selectivity(0.1))
    }
}

impl<'a> BinaryOperatorIndex for LeftAlignment<'a> {
    fn retrieve_matches(&self, lhs: &Match) -> Box<dyn Iterator<Item = Result<Match>>> {
        let mut aligned = Vec::default();

        let lhs_token = try_as_boxed_iter!(self.tok_helper.left_token_for(lhs.node));

        if let Some(lhs_token) = lhs_token {
            aligned.push(Ok(Match {
                node: lhs_token,
                anno_key: DEFAULT_ANNO_KEY.clone(),
            }));
            aligned.extend(
                self.tok_helper
                    .get_gs_left_token()
                    .get_ingoing_edges(lhs_token)
                    .map_ok(|n| Match {
                        node: n,
                        anno_key: DEFAULT_ANNO_KEY.clone(),
                    }),
            );
        }

        Box::from(
            aligned
                .into_iter()
                .map(|m| m.map_err(GraphAnnisError::from)),
        )
    }

    fn as_binary_operator(&self) -> &dyn BinaryOperatorBase {
        self
    }
}
