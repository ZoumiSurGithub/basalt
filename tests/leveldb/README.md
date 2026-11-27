# Vérification croisée avec le LevelDB de référence

`src/world/leveldb.rs` lit et écrit le format LevelDB sans bibliothèque. `tool.cc` permet de
le confronter à l'implémentation de référence (github.com/google/leveldb), dans les deux sens :

```sh
git clone https://github.com/google/leveldb && cmake -S leveldb -B build \
  -DLEVELDB_BUILD_TESTS=OFF -DLEVELDB_BUILD_BENCHMARKS=OFF && make -C build
g++ -std=c++17 tool.cc -Ileveldb/include build/libleveldb.a -lpthread -o tool

./tool make /tmp/ref                      # une base écrite par la référence
BASALT_LDB_WRITE=/tmp/basalt BASALT_LDB_READ=/tmp/ref \
  cargo test --lib cross_check -- --ignored
./tool check /tmp/basalt                  # la référence relit ce que Basalt a écrit
./tool check2 /tmp/ref                    # ... et ce que Basalt a ajouté à sa base
```

Pour lire un vrai monde du jeu : `BASALT_MOJANG_WORLD=<dossier> cargo test --lib
reads_a_world -- --ignored --nocapture`.
