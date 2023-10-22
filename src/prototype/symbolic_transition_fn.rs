use biodivine_lib_bdd::{Bdd, BddVariable, BddVariableSet};

use crate::{SymbolicDomain, VariableUpdateFnCompiled};

pub struct SymbolicTransitionFn<D: SymbolicDomain<T>, T> {
    pub transition_function: Bdd,
    penis: std::marker::PhantomData<T>,
    penis2: std::marker::PhantomData<D>,
}

impl<D: SymbolicDomain<T>, T> SymbolicTransitionFn<D, T> {
    pub fn from_update_fn_compiled(
        update_fn_compiled: &VariableUpdateFnCompiled<D, T>,
        ctx: &BddVariableSet,
        target_variable_name: &str,
    ) {
        let target_sym_dom = update_fn_compiled
            .named_symbolic_domains
            .get(target_variable_name)
            .expect("this symbolic variable/domain should be known");

        let target_sym_dom_primed = update_fn_compiled
            .named_symbolic_domains
            .get(&format!("{}'", target_variable_name))
            .expect("this symbolic variable/domain should be known");

        for (bit_answering_bdd, bdd_variable) in &update_fn_compiled.bit_answering_bdds {
            let reconstructed_target_bdd_variable_name =
                find_bdd_variables_prime(bdd_variable, target_sym_dom, target_sym_dom_primed);

            let primed_target_variable_bdd = ctx.mk_var(reconstructed_target_bdd_variable_name);
            let the_part_of_the_update_fn = primed_target_variable_bdd.iff(bit_answering_bdd);
            // todo use this; now going for lunch
        }

        todo!("##################### this is good; all the stuff was coverted to iff ")
    }
}

fn find_bdd_variables_prime<D: SymbolicDomain<T>, T>(
    target_variable: &BddVariable,
    target_sym_dom: &D,
    target_sym_dom_primed: &D,
) -> BddVariable {
    target_sym_dom
        .symbolic_variables()
        .into_iter()
        .zip(target_sym_dom_primed.symbolic_variables())
        .find_map(|(maybe_target_variable, its_primed)| {
            if maybe_target_variable == *target_variable {
                Some(its_primed)
            } else {
                None
            }
        })
        .expect("there shoudl be target_variable in target_sym_dom")
}
