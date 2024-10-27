# yumiot


### idf export
source /opt/esp-idf/export.sh
. /opt/esp-idf/export.fish

### use espflash idf 
. $HOME/esp/esp-idf/export.fish


### APP Partition size up (2MB)
cargo espflash flash --monitor --release --partition-table partitions.csv