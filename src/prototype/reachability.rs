use std::fmt::Debug;
use std::ops::Shr;
use biodivine_lib_bdd::{Bdd, BddPartialValuation};
use num_bigint::BigInt;
use crate::{SmartSystemUpdateFn, SymbolicDomain};

pub fn reachability_benchmark<D: SymbolicDomain<u8> + Debug>(sbml_path: &str) {
    let smart_system_update_fn = {
        let file = std::fs::File::open(sbml_path.clone())
            .expect("Cannot open SBML file.");
        let reader = std::io::BufReader::new(file);
        let mut xml = xml::reader::EventReader::new(reader);

        crate::find_start_of(&mut xml, "listOfTransitions")
            .expect("Cannot find transitions in the SBML file.");

        let smart_system_update_fn = SmartSystemUpdateFn::<D, u8>::try_from_xml(&mut xml)
                .expect("Loading system fn update failed.");

        smart_system_update_fn
    };

    let unit = smart_system_update_fn.unit_vertex_set();
    let system_var_count = smart_system_update_fn.get_system_variables().len();
    println!("Variables: {}, expected states {}", system_var_count, 1 << system_var_count);
    println!("Computed state count: {}", count_states(&smart_system_update_fn, &unit));
    let mut universe = unit.clone();
    while !universe.is_false() {
        let mut weak_scc = pick_state(&smart_system_update_fn, &universe);
        loop {
            let bwd_reachable = reach_bwd(&smart_system_update_fn, &weak_scc, &universe);
            let fwd_bwd_reachable = reach_fwd(&smart_system_update_fn, &bwd_reachable, &universe);

            // FWD/BWD reachable set is not a subset of weak SCC, meaning the SCC can be expanded.
            if !fwd_bwd_reachable.imp(&weak_scc).is_true() {
                println!(" + SCC increased to (states={}, size={})", count_states(&smart_system_update_fn, &weak_scc), weak_scc.size());
                weak_scc = fwd_bwd_reachable;
            } else {
                break;
            }
        }
        println!(" + Found weak SCC (states={}, size={})", count_states(&smart_system_update_fn, &weak_scc), weak_scc.size());
        // Remove the SCC from the universe set and start over.
        universe = universe.and_not(&weak_scc);
        println!(
            " + Remaining states: {}/{}",
            count_states(&smart_system_update_fn, &universe),
            count_states(&smart_system_update_fn, &unit),
        );
    }
}

/// Compute the set of vertices that are forward-reachable from the `initial` set.
///
/// The result BDD contains a vertex `x` if and only if there is a (possibly zero-length) path
/// from some vertex `x' \in initial` into `x`, i.e. `x' -> x`.
pub fn reach_fwd<D: SymbolicDomain<u8> + Debug>(system: &SmartSystemUpdateFn<D, u8>, initial: &Bdd, universe: &Bdd) -> Bdd {
    // The list of system variables, sorted in descending order (i.e. opposite order compared
    // to the ordering inside BDDs).
    let sorted_variables = system.get_system_variables();
    let mut result = initial.clone();
    println!("Start forward reachability: (states={}, size={})", count_states(system, &result), result.size());
    'fwd: loop {
        for var in sorted_variables.iter().rev() {
            let successors = system.transition_under_variable(var.as_str(), &result);

            // Should be equivalent to "successors \not\subseteq result".
            if !successors.imp(&result).is_true() {
                result = result.or(&successors);
                println!(
                    " >> (progress={:.2}%%, states={}, size={})",
                    log_percent(&result, &universe),
                    count_states(system, &result),
                    result.size()
                );
                continue 'fwd;
            }
        }

        // No further successors were computed across all variables. We are done.
        println!(" >> Done. (states={}, size={})", count_states(system, &result), result.size());
        return result;
    }
}

/// Compute the set of vertices that are backward-reachable from the `initial` set.
///
/// The result BDD contains a vertex `x` if and only if there is a (possibly zero-length) path
/// from `x` into some vertex `x' \in initial`, i.e. `x -> x'`.
pub fn reach_bwd<D: SymbolicDomain<u8> + Debug>(system: &SmartSystemUpdateFn<D, u8>, initial: &Bdd, universe: &Bdd) -> Bdd {
    let sorted_variables = system.get_system_variables();
    let mut result = initial.clone();
    println!("Start backward reachability: (states={}, size={})", count_states(system, &result), result.size());
    'bwd: loop {
        for var in sorted_variables.iter().rev() {
            let predecessors = system.predecessors_under_variable(var.as_str(), &result);

            // Should be equivalent to "predecessors \not\subseteq result".
            if !predecessors.imp(&result).is_true() {
                result = result.or(&predecessors);
                println!(
                    " >> (progress={:.2}%%, states={}, size={})",
                    log_percent(&result, &universe),
                    count_states(system, &result),
                    result.size()
                );
                continue 'bwd;
            }
        }

        // No further predecessors were computed across all variables. We are done.
        println!(" >> Done. (states={}, size={})", count_states(system, &result), result.size());
        return result;
    }
}

/// Compute a [Bdd] which represents a single (un-primed) state within the given symbolic `set`.
pub fn pick_state<D: SymbolicDomain<u8> + Debug>(system: &SmartSystemUpdateFn<D, u8>, set: &Bdd) -> Bdd {
    // Unfortunately, this is now a bit more complicated than it needs to be, because
    // we have to ignore the primed variables, but it shouldn't bottleneck anything outside of
    // truly extreme cases.
    let standard_variables = system.standard_variables();
    let valuation = set.sat_witness()
        .expect("Cannot pick state from an empty set.");
    let mut state_data = BddPartialValuation::empty();
    for var in standard_variables {
        state_data.set_value(var, valuation.value(var))
    }
    system.get_bdd_variable_set().mk_conjunctive_clause(&state_data)
}

pub fn log_percent(set: &Bdd, universe: &Bdd) -> f64 {
    set.cardinality().log2() / universe.cardinality().log2() * 100.0
}

/// Compute an (approximate) count of state in the given `set` using the encoding of `system`.
pub fn count_states<D: SymbolicDomain<u8> + Debug>(system: &SmartSystemUpdateFn<D, u8>, set: &Bdd) -> f64 {
    let symbolic_var_count = system.get_bdd_variable_set().num_vars() as i32;
    // TODO:
    //   Here we assume that exactly half of the variables are primed, which may not be true
    //   in the future, but should be good enough for now.
    assert_eq!(symbolic_var_count % 2, 0);
    let primed_vars = symbolic_var_count / 2;
    set.cardinality() / 2.0f64.powi(primed_vars)
}

pub fn count_states_exact<D: SymbolicDomain<u8> + Debug>(system: &SmartSystemUpdateFn<D, u8>, set: &Bdd) -> BigInt {
    let symbolic_var_count = system.get_bdd_variable_set().num_vars() as i32;
    // TODO:
    //   Here we assume that exactly half of the variables are primed, which may not be true
    //   in the future, but should be good enough for now.
    assert_eq!(symbolic_var_count % 2, 0);
    let primed_vars = symbolic_var_count / 2;
    set.exact_cardinality().shr(primed_vars)
}