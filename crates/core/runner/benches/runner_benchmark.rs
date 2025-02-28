use criterion::{black_box, criterion_group, criterion_main, Criterion};
use moon::{build_dep_graph, generate_project_graph, load_workspace_from};
use moon_dep_graph::DepGraph;
use moon_project_graph::ProjectGraph;
use moon_runner::Runner;
use moon_runner_context::RunnerContext;
use moon_task::Target;
use moon_test_utils::{create_sandbox_with_config, get_cases_fixture_configs};
use moon_workspace::Workspace;

fn generate_dep_graph(workspace: &Workspace, project_graph: &ProjectGraph) -> DepGraph {
    let mut dep_graph = build_dep_graph(workspace, project_graph);

    dep_graph
        .run_target(Target::parse("base:base").unwrap(), None)
        .unwrap();

    dep_graph
        .run_target(Target::parse("depsA:dependencyOrder").unwrap(), None)
        .unwrap();

    dep_graph
        .run_target(Target::parse("outputs:withDeps").unwrap(), None)
        .unwrap();

    dep_graph
        .run_target(Target::parse("passthroughArgs:c").unwrap(), None)
        .unwrap();

    dep_graph
        .run_target(Target::parse("targetScopeB:self").unwrap(), None)
        .unwrap();

    dep_graph.build()
}

pub fn runner_benchmark(c: &mut Criterion) {
    let (workspace_config, toolchain_config, projects_config) = get_cases_fixture_configs();

    let sandbox = create_sandbox_with_config(
        "cases",
        Some(&workspace_config),
        Some(&toolchain_config),
        Some(&projects_config),
    );

    c.bench_function("runner", |b| {
        b.iter(|| async {
            let mut workspace = load_workspace_from(sandbox.path()).await.unwrap();
            let project_graph = generate_project_graph(&mut workspace).unwrap();
            let dep_graph = generate_dep_graph(&workspace, &project_graph);

            black_box(
                Runner::new(workspace)
                    .run(dep_graph, project_graph, Some(RunnerContext::default()))
                    .await
                    .unwrap(),
            );
        })
    });
}

criterion_group!(runner, runner_benchmark);
criterion_main!(runner);
