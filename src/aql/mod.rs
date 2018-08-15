pub mod ast;
pub mod normalize;
pub mod operators;
pub mod parser;

use errors::*;
use operator::OperatorSpec;
use exec::nodesearch::NodeSearchSpec;
use query::conjunction::Conjunction;
use query::disjunction::Disjunction;
use std::collections::HashMap;
use std::collections::BTreeMap;

fn make_operator_spec(op: ast::BinaryOpSpec) -> Box<OperatorSpec> {
    match op {
        ast::BinaryOpSpec::Dominance(spec) => Box::new(spec),
        ast::BinaryOpSpec::Pointing(spec) => Box::new(spec),
        ast::BinaryOpSpec::Precedence(spec) => Box::new(spec),
        ast::BinaryOpSpec::Overlap(spec) => Box::new(spec),
        ast::BinaryOpSpec::IdenticalCoverage(spec) => Box::new(spec),
    }
}

pub fn parse<'a>(query_as_aql: &str) -> Result<Disjunction<'a>> {
    let ast = parser::DisjunctionParser::new().parse(query_as_aql);
    match ast {
        Ok(mut ast) => {
            // make sure AST is in DNF
            normalize::to_disjunctive_normal_form(&mut ast);

            // map all conjunctions and its literals
            // TODO: handle manually named variables
            let mut alternatives: Vec<Conjunction> = Vec::new();
            for c in ast.into_iter() {
                let mut q = Conjunction::new();
                // collect and sort all node searches according to their start position in the text
                let mut pos_to_node : BTreeMap<usize, NodeSearchSpec> = BTreeMap::default();
                for f in c.iter() {
                    if let ast::Factor::Literal(literal) = f {
                        match literal {
                            ast::Literal::NodeSearch { spec, pos } => {
                                if let Some(pos) = pos {
                                    pos_to_node.insert(pos.start, spec.clone());
                                }
                            },
                            ast::Literal::BinaryOp { lhs, rhs, .. } => {

                                if let ast::Operand::Literal{spec, pos} = lhs {
                                    pos_to_node.entry(pos.start).or_insert_with(|| spec.as_ref().clone());
                                }
                                if let ast::Operand::Literal{spec, pos} = rhs {
                                    pos_to_node.entry(pos.start).or_insert_with(|| spec.as_ref().clone());
                                }                            
                            }
                        };
                    }
                }

                // add all nodes specs in order of their start position
                let mut pos_to_node_idx: HashMap<usize, usize> = HashMap::default();
                let mut pos_to_variable : HashMap<usize, String> = HashMap::default();
                for (start_pos,node_spec) in pos_to_node.into_iter() {
                    let idx = q.add_node(node_spec, None);
                    pos_to_node_idx.insert(start_pos, idx);
                    pos_to_variable.insert(start_pos, (idx+1).to_string());
                }

                // finally add all operators

                for f in c.into_iter() {
                    if let ast::Factor::Literal(literal) = f {
                        if let ast::Literal::BinaryOp { lhs, op, rhs, .. } = literal {

                            let idx_left = match lhs {
                                ast::Operand::Literal { spec, pos } => {
                                    pos_to_node_idx.entry(pos.start).or_insert_with(|| q.add_node(spec.as_ref().clone(), None)).clone()
                                },
                                ast::Operand::NodeRef(node_ref) => {
                                    match node_ref {
                                        ast::NodeRef::ID(id) => id-1,
                                        ast::NodeRef::Name(name) => unimplemented!(), 
                                    }
                                }
                            };

                            let idx_right = match rhs {
                                ast::Operand::Literal { spec, pos } => {
                                    pos_to_node_idx.entry(pos.start).or_insert_with(|| q.add_node(spec.as_ref().clone(), None)).clone()
                                },
                                ast::Operand::NodeRef(node_ref) => {
                                    match node_ref {
                                        ast::NodeRef::ID(id) => id-1,
                                        ast::NodeRef::Name(name) => unimplemented!(), 
                                    }
                                }
                            };

                            q.add_operator(make_operator_spec(op), idx_left, idx_right);
                        }
                    }
                }

                // add the conjunction to the disjunction
                alternatives.push(q);
            }
            return Ok(Disjunction::new(alternatives));
        }
        Err(e) => {
            return Err(format!("{}", e).into());
        }
    };
}
