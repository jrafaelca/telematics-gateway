# ruptela-listener — Guía de despliegue

## Arquitectura

El sistema se compone de dos piezas independientes que se instalan en
servidores separados:

```
Dispositivos GPS
      │
      ├──→ ruptela-listener (este servidor) ─┐
      │                                      ├──→ Valkey (servidor de datos)
      └──→ ruptela-listener (réplica)  ──────┘
```

- **ruptela-listener** — recibe conexiones TCP de los dispositivos GPS.
  Se puede instalar en uno o varios servidores apuntando al mismo Valkey.
- **Valkey** — almacena los datos en streams. Se instala una sola vez en
  un servidor dedicado. Ver la sección [Valkey](#valkey--servidor-de-datos).

---

## ruptela-listener

### Requisitos

- Ubuntu 20.04 / 22.04 / 24.04 (x86-64)
- Acceso root o sudo
- Un servidor Valkey accesible en red

### Paso 1 — Crear usuario de sistema

El servicio corre bajo un usuario sin privilegios. Ejecutar una sola vez:

```bash
sudo useradd --system --no-create-home --shell /usr/sbin/nologin ruptela
```

### Paso 2 — Instalar el binario

```bash
sudo mkdir -p /opt/ruptela
sudo install -m 755 ruptela-listener /opt/ruptela/ruptela-listener
```

### Paso 3 — Configurar el servicio systemd

Crear el archivo `/etc/systemd/system/ruptela-listener.service` con el
siguiente contenido, ajustando `--redis-url` con la IP del servidor Valkey:

```ini
[Unit]
Description=Ruptela GPS TCP Listener
After=network.target

[Service]
Type=simple
User=ruptela
ExecStart=/opt/ruptela/ruptela-listener \
    --host 0.0.0.0 \
    --port 5000 \
    --redis-url redis://IP-DEL-SERVIDOR-VALKEY:6379 \
    --shards 16
Restart=always
RestartSec=5
LimitNOFILE=65536
Environment=RUST_LOG=ruptela_listener=info
StandardOutput=append:/var/log/ruptela/listener.out.log
StandardError=append:/var/log/ruptela/listener.error.log

[Install]
WantedBy=multi-user.target
```

| Parámetro     | Descripción                                                  |
|---------------|--------------------------------------------------------------|
| `--host`      | Interfaz de escucha (dejar `0.0.0.0`)                        |
| `--port`      | Puerto TCP donde conectan los dispositivos GPS               |
| `--redis-url` | URL del servidor Valkey (cambiar la IP)                      |
| `--shards`    | Número de streams (debe ser igual en todas las instancias)   |

### Paso 4 — Configurar logs

Los logs se escriben en dos archivos separados bajo `/var/log/ruptela/`:

| Archivo              | Contenido                     |
|----------------------|-------------------------------|
| `listener.out.log`   | Actividad normal del servicio |
| `listener.error.log` | Errores y advertencias        |

Crear el directorio:

```bash
sudo mkdir -p /var/log/ruptela
sudo chown ruptela:ruptela /var/log/ruptela
```

Crear el archivo `/etc/logrotate.d/ruptela-listener` para rotación diaria
(guarda 30 días, comprime los anteriores):

```
/var/log/ruptela/listener.out.log
/var/log/ruptela/listener.error.log {
    daily
    rotate 30
    compress
    delaycompress
    missingok
    notifempty
    copytruncate
    create 0640 ruptela ruptela
}
```

### Paso 5 — Iniciar el servicio

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now ruptela-listener
```

Verificar el estado:

```bash
sudo systemctl status ruptela-listener
```

Ver los logs en tiempo real:

```bash
# Actividad normal
tail -f /var/log/ruptela/listener.out.log

# Errores
tail -f /var/log/ruptela/listener.error.log
```

Consultar logs de un día específico:

```bash
# Hoy
cat /var/log/ruptela/listener.out.log
cat /var/log/ruptela/listener.error.log

# Ayer
zcat /var/log/ruptela/listener.out.log.1.gz
zcat /var/log/ruptela/listener.error.log.1.gz

# Hace dos días
zcat /var/log/ruptela/listener.out.log.2.gz
zcat /var/log/ruptela/listener.error.log.2.gz
```

### Actualización

```bash
sudo install -m 755 ruptela-listener /opt/ruptela/ruptela-listener
sudo systemctl restart ruptela-listener
sudo systemctl status ruptela-listener
```

### Desinstalación

```bash
sudo systemctl disable --now ruptela-listener
sudo rm /etc/systemd/system/ruptela-listener.service
sudo rm /etc/logrotate.d/ruptela-listener
sudo rm -rf /opt/ruptela
sudo rm -rf /var/log/ruptela
sudo userdel ruptela
sudo systemctl daemon-reload
```

---

## Valkey — Servidor de datos

Valkey es el almacén de datos compartido. Se instala **una sola vez** en un
servidor dedicado. Todos los `ruptela-listener` apuntan a este servidor.

### Requisitos

- Ubuntu 20.04 / 22.04 / 24.04 (x86-64)
- Acceso root o sudo
- Puerto 6379 accesible desde los servidores donde corre `ruptela-listener`

### Paso 1 — Instalar Valkey

```bash
sudo apt-get update
sudo apt-get install -y valkey
```

### Paso 2 — Configurar acceso por red

Por defecto Valkey solo escucha en `127.0.0.1`. Editar `/etc/valkey/valkey.conf`:

```bash
sudo nano /etc/valkey/valkey.conf
```

Cambiar la línea `bind`:

```
# Antes:
bind 127.0.0.1

# Después:
bind 0.0.0.0
```

> **Seguridad:** asegúrese de que el firewall solo permite el puerto 6379
> desde las IPs de los servidores `ruptela-listener`, no desde internet.

### Paso 3 — (Recomendado) Configurar contraseña

En el mismo archivo, descomentar y definir:

```
requirepass UNA-CONTRASEÑA-SEGURA
```

Si agrega contraseña, la URL de conexión en el `.service` debe incluirla:

```
--redis-url redis://:UNA-CONTRASEÑA-SEGURA@IP-DEL-SERVIDOR-VALKEY:6379
```

### Paso 4 — Habilitar e iniciar

```bash
sudo systemctl enable --now valkey
```

Verificar que está corriendo:

```bash
valkey-cli ping
# Debe responder: PONG
```

### Firewall (ufw)

Permitir acceso solo desde los servidores del listener:

```bash
sudo ufw allow from IP-LISTENER-1 to any port 6379
sudo ufw allow from IP-LISTENER-2 to any port 6379
sudo ufw deny 6379
```

### Verificación desde un servidor listener

```bash
valkey-cli -h IP-DEL-SERVIDOR-VALKEY ping
# Debe responder: PONG
```
