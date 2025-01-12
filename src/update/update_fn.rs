#![allow(dead_code)]

use std::{collections::HashMap, fmt::Debug};

use biodivine_lib_bdd::{
    Bdd, BddPartialValuation, BddVariable, BddVariableSet, BddVariableSetBuilder,
};

use crate::{
    expression_components::{expression::Expression, proposition::Proposition},
    symbolic_domains::symbolic_domain::{SymbolicDomain, SymbolicDomainOrd},
    update::unprocessed_variable_update_function::UnprocessedVariableUpdateFn,
};

use self::variable_update_fn::VariableUpdateFn;
use debug_ignore::DebugIgnore;

#[derive(Debug)]
pub struct SystemUpdateFn<D, T>
where
    D: SymbolicDomain<T>,
{
    /// ordered by variable name // todo add a method to get the update function by name (hash map or binary search)
    update_fns: Vec<(String, (VariableUpdateFn, D))>,
    bdd_variable_set: DebugIgnore<BddVariableSet>,
    _marker: std::marker::PhantomData<T>,
}

impl<DO, T> SystemUpdateFn<DO, T>
where
    DO: SymbolicDomainOrd<T>,
{
    pub fn from_update_fns(
        vars_and_their_update_fns: HashMap<String, UnprocessedVariableUpdateFn<T>>,
    ) -> Self {
        let named_update_fns_sorted = {
            let mut to_be_sorted = vars_and_their_update_fns.into_iter().collect::<Vec<_>>();
            to_be_sorted.sort_unstable_by_key(|(var_name, _)| var_name.clone());
            to_be_sorted
        };

        let (symbolic_domains, bdd_variable_set) = {
            let max_values = find_max_values::<DO, T>(&named_update_fns_sorted);
            let (symbolic_domains, variable_set_builder) = named_update_fns_sorted.iter().fold(
                (Vec::new(), BddVariableSetBuilder::new()),
                |(mut domains, mut variable_set), (var_name, _update_fn)| {
                    let max_value = max_values
                        .get(var_name.as_str())
                        .expect("max value always present");

                    let domain = DO::new(&mut variable_set, var_name, max_value);
                    domains.push(domain);
                    (domains, variable_set)
                },
            );

            (symbolic_domains, variable_set_builder.build())
        };

        let named_symbolic_domains = named_update_fns_sorted
            .iter()
            .zip(symbolic_domains.iter())
            .map(|((var_name, _), domain)| (var_name.as_str(), domain))
            .collect::<HashMap<_, _>>();

        let update_fns = named_update_fns_sorted
            .iter()
            .map(|(var_name, update_fn)| {
                VariableUpdateFn::from_update_fn(
                    update_fn,
                    var_name,
                    &bdd_variable_set,
                    &named_symbolic_domains,
                )
            })
            .collect::<Vec<_>>();

        let the_triple = named_update_fns_sorted
            .into_iter()
            .zip(update_fns)
            .zip(symbolic_domains)
            .map(|(((var_name, _), update_fn), domain)| (var_name, (update_fn, domain)))
            .collect::<Vec<_>>();

        Self {
            update_fns: the_triple,
            bdd_variable_set: bdd_variable_set.into(),
            _marker: std::marker::PhantomData,
        }
    }

    fn get_update_fn_and_domain_of(&self, variable_name: &str) -> Option<&(VariableUpdateFn, DO)> {
        // todo optimize using the hashtable mapper
        self.update_fns
            .iter()
            .find(|(maybe_variable_name, _)| maybe_variable_name == variable_name)
            .map(|(_, update_fn_and_domain)| update_fn_and_domain)
    }

    /// Returns a BDD that represents the set of states that are successors of
    /// any state from `source_states` under given transition variable.
    ///
    /// # Panics
    ///
    /// Panics if variable with given name is not available.
    pub fn successors_async(&self, transition_variable_name: &str, source_states_set: &Bdd) -> Bdd {
        let (update_fn, domain) = self
            .get_update_fn_and_domain_of(transition_variable_name)
            .unwrap_or_else(|| {
                panic!(
                    "no update function for variable {}; only [{}] are available",
                    transition_variable_name,
                    self.update_fns
                        .iter()
                        .map(|(var_name, _)| var_name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            });

        let each_allowed_value_bit_encoded = domain
            .decode_collection(
                &self.bdd_variable_set,
                &domain.unit_collection(&self.bdd_variable_set),
            )
            .into_iter()
            .map(|value| domain.raw_bdd_variables_encode(&value))
            .collect::<Vec<_>>();

        let unit_collection = self
            .update_fns
            .iter()
            .fold(self.bdd_variable_set.mk_true(), |acc, (_, (_, domain))| {
                acc.and(&domain.unit_collection(&self.bdd_variable_set))
            });

        let unpruned_res = each_allowed_value_bit_encoded.into_iter().fold(
            self.bdd_variable_set.mk_false(),
            |acc, val_bits| {
                let any_state_capable_of_transitioning_into_target_value = update_fn
                    .bit_answering_bdds
                    .iter()
                    .zip(&val_bits)
                    .fold(
                        self.bdd_variable_set.mk_true(),
                        |acc, ((_, bdd), val_bit)| {
                            if *val_bit {
                                acc.and(bdd)
                            } else {
                                acc.and_not(bdd)
                            }
                        },
                    )
                    .and(&unit_collection);

                let those_from_source_capable_of_transitioning_into_target_value =
                    source_states_set.and(&any_state_capable_of_transitioning_into_target_value);

                let with_forgotten_values =
                    those_from_source_capable_of_transitioning_into_target_value
                        .exists(domain.raw_bdd_variables().as_slice());

                let transitioned = with_forgotten_values.select(
                    domain
                        .raw_bdd_variables()
                        .into_iter()
                        .zip(val_bits)
                        .collect::<Vec<_>>()
                        .as_slice(),
                );

                acc.or(&transitioned)
            },
        );

        unpruned_res.and(&unit_collection)
    }

    /// Like `successors_async`, but a state that "transitions" to itself under
    /// given transition variable is not considered to be a proper successor,
    /// therefore is not included in the result (unless it is a proper successor
    /// of another state from `source_states`).
    pub fn successors_async_exclude_loops(
        &self,
        transition_variable_name: &str,
        source_states: &Bdd,
    ) -> Bdd {
        self.successors_async(
            transition_variable_name,
            &source_states
                .and(&self.those_states_capable_of_transitioning_under(transition_variable_name)),
        )
    }

    // pub fn predecessors_async
    pub fn predecessors_async(&self, transition_variable_name: &str, source_states: &Bdd) -> Bdd {
        let (update_fn, domain) = self
            .get_update_fn_and_domain_of(transition_variable_name)
            .unwrap_or_else(|| {
                panic!(
                    "no update function for variable {}; only [{}] are available",
                    transition_variable_name,
                    self.update_fns
                        .iter()
                        .map(|(var_name, _)| var_name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            });

        let each_allowed_value_bit_encoded = domain
            .decode_collection(
                &self.bdd_variable_set,
                &domain.unit_collection(&self.bdd_variable_set),
            )
            .into_iter()
            .map(|value| domain.raw_bdd_variables_encode(&value));

        each_allowed_value_bit_encoded.fold(self.bdd_variable_set.mk_false(), |acc, val_bits| {
            let filter = update_fn
                .bit_answering_bdds
                .iter()
                .zip(&val_bits)
                .map(|((bdd_variable, _bdd), bit_val)| {
                    (bdd_variable.to_owned(), bit_val.to_owned())
                })
                .collect::<Vec<_>>();

            let those_from_source_with_target_value = source_states.select(filter.as_slice());

            let possible_predecessors = those_from_source_with_target_value
                .exists(
                    filter
                        .iter()
                        .map(|(bdd_var, _)| *bdd_var)
                        .collect::<Vec<_>>()
                        .as_slice(),
                )
                .and(&domain.unit_collection(&self.bdd_variable_set)); // keep only valid states

            let unit_set = self
                .update_fns
                .iter()
                .fold(self.bdd_variable_set.mk_true(), |acc, (_, (_, domain))| {
                    acc.and(&domain.unit_collection(&self.bdd_variable_set))
                });

            let any_state_capable_of_transitioning_into_target_value =
                update_fn.bit_answering_bdds.iter().zip(&val_bits).fold(
                    // self.bdd_variable_set.mk_true(),
                    unit_set,
                    |acc, ((_, bdd), val_bit)| {
                        if *val_bit {
                            acc.and(bdd)
                        } else {
                            acc.and_not(bdd)
                        }
                    },
                );

            let predecessors =
                possible_predecessors.and(&any_state_capable_of_transitioning_into_target_value);

            acc.or(&predecessors)
        })
    }

    /// Like `predecessors_async`, but a state that "transitions" to itself under
    /// given transition variable is not considered to be a proper predecessor,
    /// therefore is not included in the result (unless it is a proper predecessor
    /// of another state from `source_states`).
    pub fn predecessors_async_exclude_loops(
        &self,
        variable_name: &str,
        source_states: &Bdd,
    ) -> Bdd {
        self.predecessors_async(variable_name, source_states)
            .and(&self.those_states_capable_of_transitioning_under(variable_name))
    }

    fn those_states_capable_of_transitioning_under(&self, _variable_name: &str) -> Bdd {
        // todo this should be stored in a field; built during construction
        todo!()
    }

    pub fn encode_one(&self, variable_name: &str, value: &T) -> Bdd {
        let (_, domain) = self
            .get_update_fn_and_domain_of(variable_name)
            .expect("unknown variable");
        domain.encode_one(&self.bdd_variable_set, value)
    }

    pub fn bdd_to_dot_string(&self, bdd: &Bdd) -> String {
        bdd.to_dot_string(&self.bdd_variable_set, false)
    }
}

struct VarInfo<D, T>
where
    D: SymbolicDomain<T>,
{
    primed_name: String,
    domain: D,
    primed_domain: D,
    transition_relation: Bdd,
    _marker: std::marker::PhantomData<T>,
}

pub struct SmartSystemUpdateFn<D, T>
where
    D: SymbolicDomain<T>,
{
    /// maps variable name to its index in the `variables_transition_relation_and_domain` vector to allow for fast access while keeping the vector sorted
    mapper: HashMap<String, usize>,
    variables_transition_relation_and_domain: Vec<(String, VarInfo<D, T>)>,
    bdd_variable_set: BddVariableSet,
    _marker: std::marker::PhantomData<T>,
}

// todo maybe use this newtype pattern to better distinguish between primed and unprimed variables (and their domains)
// /// Wrapper over a SymbolicDomain type.
// pub struct PrimedDomain<D, T>(D, std::marker::PhantomData<T>)
// where
//     D: SymbolicDomain<T>;

// use std::ops::Deref;
// impl<D, T> Deref for PrimedDomain<D, T>
// where
//     D: SymbolicDomain<T>,
// {
//     type Target = D;

//     fn deref(&self) -> &Self::Target {
//         &self.0
//     }
// }

impl<D, T> SmartSystemUpdateFn<D, T>
where
    D: SymbolicDomain<T>,
{
    /// Returns a list of [BddVariable]-s corresponding to the encoding of the standard
    /// (i.e. "un-primed") system variables.
    pub fn standard_variables(&self) -> Vec<BddVariable> {
        self.variables_transition_relation_and_domain
            .iter()
            .flat_map(|it| it.1.domain.raw_bdd_variables())
            .collect()
    }

    pub fn standard_domains(&self) -> Vec<&D> {
        self.variables_transition_relation_and_domain
            .iter()
            .map(|it| &it.1.domain)
            .collect()
    }

    pub fn standard_variables_names_and_domains(&self) -> Vec<(&str, &D)> {
        self.variables_transition_relation_and_domain
            .iter()
            .map(|(name, info)| (name.as_str(), &info.domain))
            .collect()
    }

    /// Returns a list of [BddVariable]-s corresponding to the encoding of the "primed"
    /// system variables.
    pub fn primed_variables(&self) -> Vec<BddVariable> {
        self.variables_transition_relation_and_domain
            .iter()
            .flat_map(|it| it.1.primed_domain.raw_bdd_variables())
            .collect()
    }

    pub fn get_bdd_variable_set(&self) -> &BddVariableSet {
        &self.bdd_variable_set
    }

    /// The list of system variables, sorted in ascending order (i.e. the order in which they
    /// also appear within the BDDs).1
    pub fn get_system_variables(&self) -> Vec<String> {
        self.variables_transition_relation_and_domain
            .iter()
            .map(|(var_name, _)| var_name.to_owned())
            .collect()
    }

    pub fn get_domain(&self, variable_name: &str) -> Option<&D> {
        self.mapper
            .get(variable_name)
            .map(|idx| &self.variables_transition_relation_and_domain[*idx].1.domain)
    }

    /// Compute the [Bdd] which represents the set of all vertices admissible in this
    /// [SmartSystemUpdateFn]. Normally, this would just be the `true` BDD, but if the
    /// encoding contains some invalid values, these need to be excluded.
    ///
    /// Note that this only concerns the "standard" system variables. The resulting BDD
    /// does not depend on the "primed" system variables.
    pub fn unit_vertex_set(&self) -> Bdd {
        self.variables_transition_relation_and_domain
            .iter()
            .fold(self.bdd_variable_set.mk_true(), |acc, it| {
                acc.and(&it.1.domain.unit_collection(&self.bdd_variable_set))
            })
    }

    /// Compute an (approximate) count of state in the given `set` using the encoding of `system`.
    pub fn count_states(&self, set: &Bdd) -> f64 {
        let symbolic_var_count = self.variables_transition_relation_and_domain.len() as i32;
        set.cardinality() / 2.0f64.powi(symbolic_var_count)
    }

    /// Compute a [Bdd] which represents a single (un-primed) state within the given symbolic `set`.
    pub fn pick_state_bdd(&self, set: &Bdd) -> Bdd {
        // Unfortunately, this is now a bit more complicated than it needs to be, because
        // we have to ignore the primed variables, but it shouldn't bottleneck anything outside of
        // truly extreme cases.
        let standard_variables = self
            .variables_transition_relation_and_domain
            .iter()
            .flat_map(|transition| transition.1.domain.raw_bdd_variables());
        let valuation = set
            .sat_witness()
            .expect("Cannot pick state from an empty set.");
        let mut state_data = BddPartialValuation::empty();
        for var in standard_variables {
            state_data.set_value(var, valuation.value(var))
        }
        self.bdd_variable_set.mk_conjunctive_clause(&state_data)
    }

    pub fn log_percent(set: &Bdd, universe: &Bdd) -> f64 {
        set.cardinality().log2() / universe.cardinality().log2() * 100.0
    }
}

impl<DO, T> SmartSystemUpdateFn<DO, T>
where
    DO: SymbolicDomainOrd<T>,
{
    pub fn from_update_fns(
        vars_and_their_update_fns: HashMap<String, UnprocessedVariableUpdateFn<T>>,
    ) -> Self {
        vars_and_their_update_fns.iter().for_each(|(name, _)| {
            if name.contains('\'') {
                panic!("variable name cannot contain the prime symbol \"'\" (tick) - it is reserved for inner usage")
            }
        });

        let named_update_fns_sorted = {
            let mut to_be_sorted = vars_and_their_update_fns.into_iter().collect::<Vec<_>>();
            to_be_sorted.sort_by_key(|(var_name, _)| var_name.clone());
            to_be_sorted
        };

        let (named_symbolic_domains, bdd_variable_set) = {
            let max_values = find_max_values::<DO, T>(&named_update_fns_sorted);
            let mut bdd_variable_set_builder = BddVariableSetBuilder::new();

            // let (symbolic_domains, variable_set_builder) =
            let named_symbolic_domains = named_update_fns_sorted
                .iter()
                .map(|(var_name, _)| {
                    let max_value = max_values
                        .get(var_name.as_str())
                        .expect("max value always present");

                    let original_name = var_name.clone();
                    let primed_name = format!("{}'", var_name);

                    let original =
                        DO::new(&mut bdd_variable_set_builder, &original_name, max_value);
                    let primed = DO::new(&mut bdd_variable_set_builder, &primed_name, max_value);

                    ((original_name, original), (primed_name, primed))
                })
                .collect::<Vec<_>>();

            (named_symbolic_domains, bdd_variable_set_builder.build())
        };

        let named_symbolic_domains_map = named_symbolic_domains
            .iter()
            .flat_map(|((var_name, domain), (primed_var_name, primed_domain))| {
                [
                    (var_name.as_str(), domain),
                    (primed_var_name.as_str(), primed_domain),
                ]
            })
            .collect::<HashMap<_, _>>();
        let update_fns = named_update_fns_sorted.iter().map(|(var_name, update_fn)| {
            (
                var_name,
                VariableUpdateFn::from_update_fn(
                    update_fn,
                    var_name,
                    &bdd_variable_set,
                    &named_symbolic_domains_map,
                ),
            )
        });

        let unit_set = named_symbolic_domains
            .iter()
            .fold(bdd_variable_set.mk_true(), |acc, ((_name, domain), _)| {
                acc.and(&domain.unit_collection(&bdd_variable_set))
            });

        let unprimed_var_names_and_their_primed_unit_collection = named_symbolic_domains
            .iter()
            .map(|((unprimed_var_name, _), (_, primed_domain))| {
                (
                    unprimed_var_name,
                    primed_domain.unit_collection(&bdd_variable_set),
                )
            })
            .collect::<HashMap<_, _>>();

        let relations = update_fns
            .into_iter()
            .map(|(target_variable_name, update_fn)| {
                let target_symbolic_domain_primed = *named_symbolic_domains_map
                    .get(format!("{}'", target_variable_name).as_str())
                    .expect("domain always present");

                let relation = update_fn
                    .bit_answering_bdds
                    .iter()
                    .zip(target_symbolic_domain_primed.raw_bdd_variables())
                    .fold(
                        // unit_set -> result of any `and` encodes only valid states
                        unit_set.clone(),
                        |acc, ((_bdd_var, bit_answering_bdd), bdd_var_primed)| {
                            let primed_target_variable_bdd =
                                bdd_variable_set.mk_var(bdd_var_primed);
                            let primed_bound_to_udpate =
                                primed_target_variable_bdd.iff(bit_answering_bdd);

                            acc.and(&primed_bound_to_udpate)
                        },
                    );

                let specific_primed_unit_set = unprimed_var_names_and_their_primed_unit_collection
                    .get(target_variable_name)
                    .expect("always present");

                // ensure output only valid values
                relation.and(specific_primed_unit_set)
            })
            .collect::<Vec<_>>();

        let variables_transition_relation_and_domain = named_symbolic_domains
            .into_iter()
            .zip(relations)
            .map(
                |(((var_name, domain), (primed_var_name, primed_domain)), relation_bdd)| {
                    (
                        var_name,
                        VarInfo {
                            primed_name: primed_var_name,
                            domain,
                            primed_domain,
                            transition_relation: relation_bdd,
                            _marker: std::marker::PhantomData,
                        },
                    )
                },
            )
            .collect::<Vec<_>>();

        let mapper = variables_transition_relation_and_domain
            .iter()
            .enumerate()
            .fold(HashMap::new(), |mut acc, (idx, (var_name, _))| {
                acc.insert(var_name.to_owned(), idx);
                acc
            });

        Self {
            mapper,
            variables_transition_relation_and_domain,
            bdd_variable_set,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn successors_async(&self, transition_variable_name: &str, source_states_set: &Bdd) -> Bdd {
        let VarInfo {
            transition_relation,
            domain: target_domain,
            primed_domain,
            ..
        } = self
            .get_transition_relation_and_domain(transition_variable_name)
            .expect("unknown variable");

        let source_states_transition_relation = source_states_set.and(transition_relation);

        let forgor_old_val =
            source_states_transition_relation.exists(target_domain.raw_bdd_variables().as_slice());

        target_domain
            .raw_bdd_variables()
            .into_iter()
            .zip(primed_domain.raw_bdd_variables())
            .fold(forgor_old_val, |mut acc, (unprimed, primed)| {
                unsafe { acc.rename_variable(primed, unprimed) };
                acc
            })
    }

    /// Like `successors_async`, but a state that "transitions" to itself under
    /// given transition variable is not considered to be a proper successor,
    /// therefore is not included in the result (unless it is a proper successor
    /// of another state from `source_states`).
    pub fn successors_async_exclude_loops(
        &self,
        _transition_variable_name: &str,
        _source_states: &Bdd,
    ) -> Bdd {
        // todo better to directly construct the specific no_loop_transition_bdd during construction
        // todo probably have some common, underlying method that would accept the transition bdd
        // todo the two public methods would then just pass in the proper transition bdd
        // self.successors_async(
        //     transition_variable_name,
        //     &source_states
        //         .and(&self.those_states_capable_of_transitioning_under(transition_variable_name)),
        // )
        todo!()
    }

    pub fn predecessors_async(
        &self,
        transition_variable_name: &str,
        source_states_set: Bdd, // todo inconsistent with succs api; but `rename_variable` requires ownership
    ) -> Bdd {
        let VarInfo {
            transition_relation,
            domain: target_domain,
            primed_domain,
            ..
        } = self
            .get_transition_relation_and_domain(transition_variable_name)
            .expect("unknown variable");

        let source_states_primed_set = target_domain
            .raw_bdd_variables()
            .into_iter()
            .zip(primed_domain.raw_bdd_variables())
            .rev() // it's magic
            .fold(source_states_set, |mut acc, (unprimed, primed)| {
                unsafe { acc.rename_variable(unprimed, primed) };
                acc
            });

        let source_states_transition_relation = source_states_primed_set.and(transition_relation);

        source_states_transition_relation.exists(primed_domain.raw_bdd_variables().as_slice())
    }

    /// Like `predecessors_async`, but a state that "transitions" to itself under
    /// given transition variable is not considered to be a proper predecessor,
    /// therefore is not included in the result (unless it is a proper predecessor
    /// of another state from `source_states`).
    pub fn predecessors_async_exclude_loops(
        &self,
        _variable_name: &str,
        _source_states: &Bdd,
    ) -> Bdd {
        // todo better to directly construct the specific no_loop_transition_bdd during construction
        // todo probably have some common, underlying method that would accept the transition bdd
        // todo the two public methods would then just pass in the proper transition bdd
        // self.predecessors_async(transition_variable_name, source_states)
        //     .and(&self.those_states_capable_of_transitioning_under(transition_variable_name))
        todo!()
    }

    fn get_transition_relation_and_domain(&self, variable_name: &str) -> Option<&VarInfo<DO, T>> {
        self.mapper
            .get(variable_name)
            .map(|idx| &self.variables_transition_relation_and_domain[*idx].1)
    }

    fn those_states_capable_of_transitioning_under(&self, _variable_name: &str) -> Bdd {
        // todo this should be stored in a field; built during construction
        todo!()
    }

    pub fn encode_one(&self, variable_name: &str, value: &T) -> Bdd {
        let VarInfo { domain, .. } = self
            .get_transition_relation_and_domain(variable_name)
            .expect("unknown variable");
        domain.encode_one(&self.bdd_variable_set, value)
    }

    pub fn bdd_to_dot_string(&self, bdd: &Bdd) -> String {
        bdd.to_dot_string(&self.bdd_variable_set, false)
    }
}

fn find_bdd_variables_prime<D, T>(
    target_variable: &BddVariable,
    target_sym_dom: &D,
    target_sym_dom_primed: &D,
) -> BddVariable
where
    D: SymbolicDomain<T>,
{
    target_sym_dom
        .raw_bdd_variables()
        .into_iter()
        .zip(target_sym_dom_primed.raw_bdd_variables())
        .find_map(|(maybe_target_var, var_primed)| {
            (maybe_target_var == *target_variable).then_some(var_primed)
        })
        .expect("should be present")
}

fn find_max_values<DO, T>(
    vars_and_their_update_fns: &[(String, UnprocessedVariableUpdateFn<T>)],
) -> HashMap<&str, &T>
where
    DO: SymbolicDomainOrd<T>,
{
    let max_outputs =
        vars_and_their_update_fns
            .iter()
            .fold(HashMap::new(), |mut acc, (var_name, update_fn)| {
                let max_value = update_fn
                    .terms
                    .iter()
                    .map(|(val, _)| val)
                    .chain(Some(&update_fn.default))
                    .max_by(|x, y| DO::cmp(x, y))
                    .expect("default value always present");
                // no balls
                // // SAFETY: there is always at least the default value
                // let max_value = unsafe { max_value_option.unwrap_unchecked() };
                acc.insert(var_name.as_str(), max_value);
                acc
            });

    // the following step is necessary on "faulty" datasets, that compare variables
    //  with values that are out of the domain of the variable
    //  e.g. `target eq 999` when (integer) `target` has max value 2
    vars_and_their_update_fns
        .iter()
        .flat_map(|(_var_name, update_fn)| update_fn.terms.iter().map(|(_, expr)| expr))
        .fold(max_outputs, |mut acc, expr| {
            update_max::<DO, T>(&mut acc, expr);
            acc
        })
}

fn update_max<'a, DO, T>(acc: &mut HashMap<&'a str, &'a T>, expr: &'a Expression<T>)
where
    DO: SymbolicDomainOrd<T>,
{
    match expr {
        Expression::Terminal(proposition) => {
            update_from_proposition::<DO, T>(acc, proposition);
        }
        Expression::Not(expression) => {
            update_max::<DO, T>(acc, expression);
        }
        Expression::And(clauses) | Expression::Or(clauses) => {
            clauses
                .iter()
                .for_each(|clause| update_max::<DO, T>(acc, clause));
        }
        Expression::Xor(lhs, rhs) | Expression::Implies(lhs, rhs) => {
            update_max::<DO, T>(acc, lhs);
            update_max::<DO, T>(acc, rhs);
        }
    }
}

fn update_from_proposition<'a, DO, T>(
    acc: &mut HashMap<&'a str, &'a T>,
    proposition: &'a Proposition<T>,
) where
    DO: SymbolicDomainOrd<T>,
{
    let Proposition {
        variable, value, ..
    } = proposition;

    acc.entry(variable.as_str())
        .and_modify(|old_val| {
            if DO::cmp(old_val, value) == std::cmp::Ordering::Less {
                *old_val = value
            }
        })
        .or_insert(value);
}

pub mod variable_update_fn {
    use std::collections::HashMap;

    use biodivine_lib_bdd::{Bdd, BddVariable, BddVariableSet};

    use crate::{
        expression_components::{
            expression::Expression,
            proposition::{ComparisonOperator as CmpOp, Proposition},
        },
        symbolic_domains::symbolic_domain::SymbolicDomainOrd,
        update::unprocessed_variable_update_function::UnprocessedVariableUpdateFn as UnprocessedFn,
    };

    #[derive(Debug)]
    pub struct VariableUpdateFn {
        pub bit_answering_bdds: Vec<(BddVariable, Bdd)>,
    }

    impl VariableUpdateFn {
        /// target_variable_name is a key in named_symbolic_domains
        pub fn from_update_fn<DO, T>(
            update_fn: &UnprocessedFn<T>,
            target_variable_name: &str,
            bdd_variable_set: &BddVariableSet,
            named_symbolic_domains: &HashMap<&str, &DO>,
        ) -> Self
        where
            DO: SymbolicDomainOrd<T>,
        {
            let UnprocessedFn { terms, default, .. } = update_fn;

            let (outputs, bdd_conds): (Vec<_>, Vec<_>) = terms
                .iter()
                .map(|(val, match_condition)| {
                    let match_condition_bdd = bdd_from_expression(
                        match_condition,
                        named_symbolic_domains,
                        bdd_variable_set,
                    );
                    (val, match_condition_bdd)
                })
                .chain(Some((default, bdd_variable_set.mk_true())))
                .unzip();

            let (_, values_mutally_exclusive_terms) = bdd_conds.into_iter().fold(
                (bdd_variable_set.mk_false(), Vec::new()),
                |(seen_states, mut acc), term_bdd| {
                    let mutually_exclusive_bdd = term_bdd.and(&seen_states.not());

                    let updated_ctx_seen_states = seen_states.or(&term_bdd);
                    acc.push(mutually_exclusive_bdd);

                    (updated_ctx_seen_states, acc)
                },
            );

            let target_domain = named_symbolic_domains
                .get(target_variable_name)
                .expect("must know the domain of the target variable");

            let bit_matrix = outputs
                .into_iter()
                .map(|output| target_domain.raw_bdd_variables_encode(output))
                .collect::<Vec<_>>();

            let bit_answering_bdds = (0..bit_matrix[0].len()).map(|bit_idx| {
                (0..bit_matrix.len()).fold(bdd_variable_set.mk_false(), |acc, row_idx| {
                    if bit_matrix[row_idx][bit_idx] {
                        acc.or(&values_mutally_exclusive_terms[row_idx])
                    } else {
                        acc
                    }
                })
            });

            Self {
                bit_answering_bdds: target_domain
                    .raw_bdd_variables()
                    .into_iter()
                    .zip(bit_answering_bdds)
                    .collect(),
            }
        }
    }

    fn bdd_from_expression<DO, T>(
        expression: &Expression<T>,
        named_symbolic_domains: &HashMap<&str, &DO>,
        bdd_variable_set: &BddVariableSet,
    ) -> Bdd
    where
        DO: SymbolicDomainOrd<T>,
    {
        match expression {
            Expression::Terminal(proposition) => {
                bdd_from_proposition(proposition, named_symbolic_domains, bdd_variable_set)
            }
            Expression::Not(expression) => {
                bdd_from_expression(expression, named_symbolic_domains, bdd_variable_set).not()
            }
            Expression::And(clauses) => {
                clauses
                    .iter()
                    .fold(bdd_variable_set.mk_true(), |acc, clausule| {
                        acc.and(&bdd_from_expression(
                            clausule,
                            named_symbolic_domains,
                            bdd_variable_set,
                        ))
                    })
            }
            Expression::Or(clauses) => {
                clauses
                    .iter()
                    .fold(bdd_variable_set.mk_false(), |acc, clausule| {
                        acc.or(&bdd_from_expression(
                            clausule,
                            named_symbolic_domains,
                            bdd_variable_set,
                        ))
                    })
            }
            Expression::Xor(lhs, rhs) => {
                let lhs = bdd_from_expression(lhs, named_symbolic_domains, bdd_variable_set);
                let rhs = bdd_from_expression(rhs, named_symbolic_domains, bdd_variable_set);
                lhs.xor(&rhs)
            }
            Expression::Implies(lhs, rhs) => {
                let lhs = bdd_from_expression(lhs, named_symbolic_domains, bdd_variable_set);
                let rhs = bdd_from_expression(rhs, named_symbolic_domains, bdd_variable_set);
                lhs.imp(&rhs)
            }
        }
    }

    fn bdd_from_proposition<DO, T>(
        proposition: &Proposition<T>,
        named_symbolic_domains: &HashMap<&str, &DO>,
        bdd_variable_set: &BddVariableSet,
    ) -> Bdd
    where
        DO: SymbolicDomainOrd<T>,
    {
        let target_vars_domain = named_symbolic_domains.get(proposition.variable.as_str()).unwrap_or_else(
            || panic!(
                "Symbolic domain for variable {} should be avilable, but is not; domains available only for variables [{}]",
                proposition.variable,
                named_symbolic_domains.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        );

        match proposition.comparison_operator {
            CmpOp::Eq => target_vars_domain.encode_one(bdd_variable_set, &proposition.value),
            CmpOp::Neq => target_vars_domain
                .encode_one(bdd_variable_set, &proposition.value)
                .not(),
            CmpOp::Lt => target_vars_domain.encode_lt(bdd_variable_set, &proposition.value),
            CmpOp::Leq => target_vars_domain.encode_le(bdd_variable_set, &proposition.value),
            CmpOp::Gt => target_vars_domain.encode_gt(bdd_variable_set, &proposition.value),
            CmpOp::Geq => target_vars_domain.encode_ge(bdd_variable_set, &proposition.value),
        }
    }
}
