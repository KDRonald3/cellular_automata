use rayon::prelude::*;
use std::collections::HashMap;
use std::io::stdin;
// use std::process::Output;

const ONE: i32 = 1;
const ZERO: i32 = 0;

struct Structure {
    structure: HashMap<i32, Vec<i32>>,
}

// impl Structure {
//     fn get_slice(&self, generation: &i32, center: &i32) -> Result<&[i32], String> {
//         let slice = match self.structure.get(generation) {
//             Some(value) => { if value.len()> *center as usize +1{
//                 Ok(value[*center as usize - 1..*center as usize +2]);
//             }

//             },
//             None => Err(format!(
//                 "could not get slice at {} for generation {}",
//                 *center, *generation
//             )),
//         };

//         return slice;
//     }
// }

struct Rule {
    number: i32,
    rule: HashMap<Vec<i32>, i32>,
}

impl Rule {
    fn base_10_to_2(&self, number_base_10: &i32, length: usize) -> Vec<i32> {
        let mut number = *number_base_10;
        let mut number_base_2 = Vec::new();

        while number != ZERO {
            number_base_2.insert(ZERO as usize, number % 2);
            number = (number - number % 2) / 2;
        }
        while number_base_2.len() < length {
            number_base_2.insert(ZERO as usize, ZERO);
        }

        return number_base_2;
    }

    fn create_rule(&mut self) {
        let rule_number_base_2 = self.base_10_to_2(&self.number, 8);
        for i in ZERO..8 {
            self.rule
                .insert(self.base_10_to_2(&i, 3), rule_number_base_2[i as usize]);
        }
    }
}
struct InitialCondition {
    size: usize,
    state: Vec<i32>,
}

struct CellularAutomaton {
    rule: Rule,
    size: i32,
    final_generation: i32,
    structure: Structure,
}

impl CellularAutomaton {
    fn run(&mut self, mut initial_condition: InitialCondition, final_generation: i32) {
        let number_of_threads = initial_condition.size;
        let mut structure = Structure {
            structure: HashMap::new(),
        };
        initial_condition.state.insert(ZERO as usize, ONE);
        initial_condition.state.push(ONE);
        structure.structure.insert(ZERO, initial_condition.state);

        for generation in ZERO..final_generation {
            structure.structure.insert(
                generation + 1,
                (ZERO..number_of_threads as i32)
                    .into_par_iter()
                    .map(|i| {
                        let slice: &[i32] = match structure.structure.get(&generation) {
                            Some(vec_slice) => &vec_slice[i as usize..i as usize + 3],
                            None => &vec![1, 1, 1][0..3],
                        };

                        match self.rule.rule.get(slice) {
                            Some(value) => *value,
                            None => 9,
                        }
                    })
                    .collect(),
            );

            structure.structure.entry(generation + 1).and_modify(|f| {
                f.insert(ZERO as usize, ONE);
                f.push(ONE);
            });
        }

        self.size = number_of_threads as i32;
        self.structure = structure;
    }
}

fn main() {
    let mut initial_state_string = String::new();
    let mut initial_state_vec: Vec<i32> = Vec::new();
    let mut initial_condition: InitialCondition = InitialCondition {
        size: (ZERO as usize),
        state: (Vec::new()),
    };
    let mut rule_number_string: String = String::new();
    let mut final_generation_string: String = String::new();

    println!("Enter initial state");
    stdin()
        .read_line(&mut initial_state_string)
        .expect("fialed to read initial state");
    initial_state_string = initial_state_string.split("\r\n").collect();

    for state in initial_state_string.chars() {
        match state.to_digit(10) {
            Some(n) => {
                initial_state_vec.push(n as i32);
                println!("{}", n);
            }
            None => continue,
        };
    }

    initial_condition.size = initial_state_vec.len();
    initial_condition.state = initial_state_vec;

    println!("Enter rule number: ");
    stdin()
        .read_line(&mut rule_number_string)
        .expect("could not read rule number");

    let rule_number = rule_number_string
        .split("\r\n")
        .collect::<String>()
        .parse::<i32>()
        .unwrap();

    let mut rule = Rule {
        number: rule_number,
        rule: HashMap::new(),
    };
    rule.create_rule();

    println!("Enter final generation: ");
    stdin()
        .read_line(&mut final_generation_string)
        .expect("failed to read generation");
    let final_generation: i32 = final_generation_string
        .split("\r\n")
        .collect::<String>()
        .parse::<i32>()
        .unwrap();

    let mut cellular_automaton: CellularAutomaton = CellularAutomaton {
        rule,
        size: (0),
        final_generation,
        structure: Structure {
            structure: HashMap::new(),
        },
    };

    cellular_automaton.run(initial_condition, final_generation);
    for i in 0..cellular_automaton.final_generation {
        println!("{:?}", cellular_automaton.structure.structure.get(&i))
    }
}
