use ascii_dag::graph::DAG;

fn main() {
    let dag = DAG::from_edges(&[(1, "A"), (2, "B"), (3, "C")], &[(1, 2), (2, 3)]);

    println!("{}", dag.render());
}
