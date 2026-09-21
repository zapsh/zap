#!/usr/bin/env bash



export PATH=$PATH:/usr/local/mysql/bin

# Check if user is root
if [ $(id -u) != "0" ];then
  echo "Please run this script as root"
  exit 1
fi

DB_TYPE="mysql"
DB_DIR="/usr/local/mysql"
while :;do
  read -p "Do you want to reset the password of mysql root user? (y/n) " yn
  case $yn in
    [Yy]* ) break;;
    [Nn]* ) exit;;
    * ) echo "Please answer y or n.";;
  esac
done

while :;do
    read -p "请输入MySQL / MariaDB的安装目录 (默认:/usr/local/mysql): " DB_DIR
    if [ -z "$DB_DIR" ];then
        DB_DIR="/usr/local/mysql"
        break
    fi
    if [ ! -d "$DB_DIR" ];then
        echo "目录不存在，请重新输入"
        continue
    fi
done



if [ -f "$DB_DIR/bin/mariadb" ];then
    DB_TYPE="mariadb"
fi

DB_PASSWORD=$(< /dev/urandom tr -dc A-Za-z0-9_ | head -c9)
while :;do
    read -p "请输入MySQL / MariaDB的root用户密码 (默认: ${DB_PASSWORD}): " DB_PASSWORD
    if [ -z "$DB_PASSWORD" ];then
        DB_PASSWORD="${DB_PASSWORD}"
        break
    else
        break
    fi
done


echo "============================="
echo "Stop MySQL / MariaDB service..."
if command -v systemctl >/dev/null 2>&1;then
    if [ $DB_TYPE = "mysql" ];then
        systemctl stop mysql.service
    elif [ $DB_TYPE = "mariadb" ];then
        systemctl stop mariadb.service
    fi
elif command -v service >/dev/null 2>&1;then
    if [ $DB_TYPE = "mysql" ];then
        service mysql stop
    elif [ $DB_TYPE = "mariadb" ];then
        service mariadb stop
    fi
fi

echo "Start MySQL / MariaDB service... SKIP GRANT TABLES"
${DB_DIR}/bin/mysqld_safe --skip-grant-tables >/dev/null 2>&1 &
echo "sleep 5s"
sleep 5
DB_PID=`cat ${DB_DIR}/data/mysql.pid`
${DB_DIR}/bin/mysql -uroot -e "FLUSH PRIVILEGES;ALTER USER 'root'@'localhost' IDENTIFIED BY '${DB_PASSWORD}';"

if [ $? = 0 ];then
    echo "MySQL / MariaDB password reset successful!"
else
    echo "Failed!"
    exit 1
fi
echo "MySQL / MariaDB PID: $DB_PID , Stop MySQL / MariaDB service..."
kill $DB_PID

sleep 5
echo "Restart MySQL / MariaDB service..."
if command -v systemctl >/dev/null 2>&1;then
    if [ $DB_TYPE = "mysql" ];then
        systemctl restart mysql.service
        systemctl status mysql.service
    elif [ $DB_TYPE = "mariadb" ];then
        systemctl restart mariadb.service
        systemctl status mariadb.service
    fi
elif command -v service >/dev/null 2>&1;then
    if [ $DB_TYPE = "mysql" ];then
        service mysql restart
        service mysql status
    elif [ $DB_TYPE = "mariadb" ];then
        service mariadb restart
        service mariadb status
    fi
fi


