use crate::{Expression, SymbolicDomain, UnaryIntegerDomain, UpdateFn};

use std::collections::HashMap;

#[allow(unused_imports)] // todo refactor & remove ignore
use biodivine_lib_bdd::{
    Bdd, BddPartialValuation, BddVariable, BddVariableSet, BddVariableSetBuilder,
};

use super::expression::Proposition;

pub struct UpdateFnBdd {
    pub terms: Vec<(u16, Bdd)>,
    pub named_symbolic_domains: HashMap<String, UnaryIntegerDomain>,
}

// todo UpdateFn should be made obsolete, it is just an intermediate representation of what should eventually be UpdateFnBdd
impl From<UpdateFn> for UpdateFnBdd {
    fn from(source: UpdateFn) -> Self {
        // todo how to get the higest value of a variable? could not be read from the xml/UpdateFn
        let hardcoded_max_var_value = 2;

        let mut bdd_variable_set_builder = BddVariableSetBuilder::new();

        // let mut symbolic_domains = Vec::<UnaryIntegerDomain>::new();
        let mut named_symbolic_domains = HashMap::<String, UnaryIntegerDomain>::new();

        source.input_vars_names.iter().for_each(|name| {
            let var = UnaryIntegerDomain::new(
                &mut bdd_variable_set_builder,
                name,
                hardcoded_max_var_value,
            );

            named_symbolic_domains.insert(name.clone(), var);
        });

        let mut bdd_variable_set = bdd_variable_set_builder.build();
        let mut terms = Vec::<(u16, Bdd)>::new();
        source.terms.iter().for_each(|(val, expr)| {
            let bdd = bdd_from_expr(
                expr.to_owned(),
                &named_symbolic_domains,
                &mut bdd_variable_set,
            );

            terms.push((*val, bdd));
        });

        Self {
            terms,
            named_symbolic_domains,
        }
    }
}

fn bdd_from_expr(
    expr: &Expression,
    symbolic_domains: &HashMap<String, UnaryIntegerDomain>,
    bdd_variable_set: &mut BddVariableSet,
) -> Bdd {
    match expr {
        Expression::Terminal(prop) => prop_to_bdd(prop.clone(), symbolic_domains, bdd_variable_set),
        Expression::Not(expr) => {
            let bdd = bdd_from_expr(expr, symbolic_domains, bdd_variable_set);
            bdd.not()
        }
        Expression::And(lhs, rhs) => {
            let lhs = bdd_from_expr(lhs, symbolic_domains, bdd_variable_set);
            let rhs = bdd_from_expr(rhs, symbolic_domains, bdd_variable_set);
            lhs.and(&rhs)
        }
        Expression::Or(lhs, rhs) => {
            let lhs = bdd_from_expr(lhs, symbolic_domains, bdd_variable_set);
            let rhs = bdd_from_expr(rhs, symbolic_domains, bdd_variable_set);
            lhs.or(&rhs)
        }
        Expression::Xor(lhs, rhs) => {
            let lhs = bdd_from_expr(lhs, symbolic_domains, bdd_variable_set);
            let rhs = bdd_from_expr(rhs, symbolic_domains, bdd_variable_set);
            lhs.xor(&rhs)
        }
        Expression::Implies(lhs, rhs) => {
            let lhs = bdd_from_expr(lhs, symbolic_domains, bdd_variable_set);
            let rhs = bdd_from_expr(rhs, symbolic_domains, bdd_variable_set);
            lhs.imp(&rhs)
        }
    }
}

fn prop_to_bdd(
    prop: Proposition,
    symbolic_domains: &HashMap<String, UnaryIntegerDomain>,
    bdd_variable_set: &mut BddVariableSet,
) -> Bdd {
    println!("prop ci: <{:?}>", prop.ci);
    println!("domains keys: {:?}", symbolic_domains.keys());

    let var = symbolic_domains.get(&prop.ci).unwrap();
    let val = prop.cn;

    match prop.cmp {
        super::expression::CmpOp::Eq => var.encode_one(bdd_variable_set, &(val as u8)),
        super::expression::CmpOp::Neq => var.encode_one(bdd_variable_set, &(val as u8)).not(),
        super::expression::CmpOp::Lt => lt(var, bdd_variable_set, val),
        super::expression::CmpOp::Leq => leq(var, bdd_variable_set, val),
        super::expression::CmpOp::Gt => leq(var, bdd_variable_set, val).not(),
        super::expression::CmpOp::Geq => lt(var, bdd_variable_set, val).not(),
    }
}

fn lt(
    symbolic_domain: &UnaryIntegerDomain,
    bdd_variable_set: &mut BddVariableSet,
    lower_than_this: u16,
) -> Bdd {
    let mut bdd = symbolic_domain.empty_collection(bdd_variable_set);

    (0..lower_than_this).for_each(|i| {
        let bdd_i = symbolic_domain.encode_one(bdd_variable_set, &(i as u8));
        bdd = bdd.or(&bdd_i);
    });

    bdd
}

fn leq(
    symbolic_domain: &UnaryIntegerDomain,
    bdd_variable_set: &mut BddVariableSet,
    lower_or_same_as_this: u16,
) -> Bdd {
    let mut bdd = symbolic_domain.empty_collection(bdd_variable_set);

    (0..(lower_or_same_as_this + 1)).for_each(|i| {
        let bdd_i = symbolic_domain.encode_one(bdd_variable_set, &(i as u8));
        bdd = bdd.or(&bdd_i);
    });

    bdd
}

mod tests {
    use biodivine_lib_bdd::{BddPartialValuation, BddValuation};

    use crate::{SymbolicDomain, UpdateFnBdd};

    #[test]
    pub fn test_update_fn() {
        let update_fn = get_update_fn();

        println!("update fn: {:?}", update_fn);

        let update_fn_bdd: UpdateFnBdd = update_fn.into();

        let domain = update_fn_bdd.named_symbolic_domains.get("Mdm2nuc").unwrap();

        let mut valuation = BddPartialValuation::empty();

        domain.encode_bits(&mut valuation, &1);

        let bdd = update_fn_bdd.terms[0].1.clone();

        println!(
            "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@var is represented by {:?} in bdd",
            domain.symbolic_variables()
        );

        let actual_valuation = BddValuation::new({
            let mut vec = vec![false; domain.symbolic_variables().len()];
            vec[0] = true;
            vec
        });

        let bdd_res = bdd.eval_in(&actual_valuation);

        println!("bdd res: {:?}", bdd_res);
    }

    fn get_update_fn() -> super::UpdateFn {
        use std::fs::File;
        use std::io::BufReader;

        let file = File::open("data/dataset.sbml").expect("cannot open file");
        let file = BufReader::new(file);

        let mut xml = xml::reader::EventReader::new(file);

        let mut indent = 0;
        loop {
            match xml.next() {
                Ok(xml::reader::XmlEvent::StartElement { name, .. }) => {
                    println!("{}<{:?}>", "  ".repeat(indent), name);
                    indent += 1;
                    if name.local_name == "transition" {
                        let update_fn = super::UpdateFn::try_from_xml(&mut xml);
                        return update_fn.unwrap();
                    }
                }
                Ok(xml::reader::XmlEvent::EndElement { .. }) => {
                    indent -= 1;
                }
                Ok(xml::reader::XmlEvent::EndDocument) => {
                    panic!()
                }
                Err(_) => {
                    panic!()
                }
                _ => {}
            }
        }
    }
}