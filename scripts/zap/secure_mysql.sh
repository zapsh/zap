# SPDX-License-Identifier: AGPL-3.0-only
# remove anonymous users
DELETE FROM mysql.global_priv WHERE User='';

#remote root 
DELETE FROM mysql.global_priv WHERE User='root' AND Host NOT IN ('localhost', '127.0.0.1', '::1');

# remove test database
DROP DATABASE IF EXISTS test;

#Removing privileges on test database
DELETE FROM mysql.db WHERE Db='test' OR Db='test\\_%'

#flush privileges
FLUSH PRIVILEGES;