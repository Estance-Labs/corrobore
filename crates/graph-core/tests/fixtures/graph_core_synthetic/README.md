# Graph-core synthetic functional dataset

Dataset cible : **graph-core** pour `Noetance-Labs/corrobore`.

Ce pack sert à valider fonctionnellement la livraison du graph-core sans figer l’implémentation interne. Les fixtures doivent être exécutées via l’API publique de `graph_core`, pas via les modules privés ni la structure de stockage.

## Contenu

- `fixtures/entities.json` : nœuds synthétiques CTI/FIMI/crisis-like, mais traités comme labels/propriétés génériques.
- `fixtures/relationships.json` : relations synthétiques entre les nœuds.
- `fixtures/happy_path_operations.json` : séquence déclarative pour construire un graphe complet.
- `expected/expected_state_after_happy_path.json` : état observable attendu après la séquence happy path.
- `fixtures/functional_scenarios.json` : scénarios Given / When / Then pour tests fonctionnels.
- `fixtures/error_scenarios.json` : scénarios d’erreurs typées attendues.
- `fixtures/property_roundtrip.json` : couverture des propriétés MVP.
- `fixtures/validation_cases.json` : validation confidence, labels, relationship types et IDs manquants.
- `docs/coverage_matrix.csv` : mapping dataset → issues de l’epic.
- `templates/rust_dataset_harness_template.rs` : squelette optionnel de harness Rust.

## Règles d’utilisation

1. Les champs `ref` sont des références logiques de dataset, pas des `NodeId` / `RelationshipId` attendus.
2. Le harness doit créer les records, stocker les IDs générés, puis résoudre les refs lors des assertions.
3. Ne pas assert l’ordre de retour de `list_nodes`, `outgoing`, `incoming` ou `relationships_between` sauf si l’API le définit explicitement.
4. Les labels `ThreatActor`, `Malware`, `FIMIIncident`, `Narrative`, etc. ne doivent pas devenir des règles métier dans `graph-core`.
5. Les tombstones doivent être cachés par les reads par défaut, mais visibles via les APIs d’historique de versions.
6. Les erreurs doivent être matchées par variantes typées, pas par texte.

## Commande de validation cible

```bash
cargo test -p graph-core
```

## Couverture fonctionnelle

Le dataset couvre :

- identifiants typés et IDs manquants syntaxiquement valides ;
- propriétés scalar/list : null, bool, integer, float, string et listes homogènes ;
- confidence bornée inclusive `[0.0, 1.0]` avec rejet de `NaN` ;
- lifecycle node : create, get, list, update, tombstone, versions ;
- lifecycle relationship : create, get, update, tombstone, versions ;
- adjacency : outgoing, incoming, between ;
- erreurs typées : not found, source/target missing, already tombstoned ;
- non-régression sur l’API publique sans hypothèse de stockage full-graph.

## Pourquoi il y a des noms CTI/FIMI

Ils sont là pour garder un dataset réaliste pour la suite du projet, mais Epic 0001 reste un core générique. Pour Epic 0001, ces noms sont uniquement des chaînes dans des labels/propriétés.
