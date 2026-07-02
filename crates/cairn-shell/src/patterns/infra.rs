//! Infrastructure patterns (Category::Infra). docker, kubectl, helm, terraform, aws, gcloud.

use crate::registry::Pattern;

pub const DOCKER_PS: Pattern = Pattern {
    name: "docker-ps",
    category: crate::category::Category::Infra,
    matchers: &["docker", "ps"],
    // docker ps prints aligned columns; headers + non-blank container rows are enough.
    keep: Some(&[
        "CONTAINER ID",
        "IMAGE",
        "STATUS",
        "PORTS",
        "NAMES",
        "Up",
        "Exited",
    ]),
    drop: Some(&["CONTAINER ID   IMAGE"]),
};

pub const KUBECTL_GET: Pattern = Pattern {
    name: "kubectl-get",
    category: crate::category::Category::Infra,
    matchers: &["kubectl", "get"],
    keep: Some(&["NAME", "STATUS", "READY", "AGE", "Error", "Warning"]),
    drop: None,
};
