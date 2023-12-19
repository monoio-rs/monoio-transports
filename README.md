# Monoio Transporter

A set of transporters implment on Monoio runtime.

## Basic Idea

- The streaming connectors can be directly used by application
- The streaming connectors can be used as pool connector by application
- The streaming connectors can be directly used by the message based connectors
- The streaming connectors can be used as pool connector by the message based connectors

``` plain
+---------------+                   +-----------------+
| HttpConnector +----+         +----+ ThriftConnector |
+--+------------+    |         |    +--------+--------+
   |                 |         |             |
   |                 |         |             |
   |                 v         v             |
   |             +---+---------+---+         |
   |      +------+ PooledConnector +------+  |
   |      |      +--------+--------+      |  |
   |      |               |               |  |
   |      |               |               |  |
   v      v               v               v  v
+--+------+----+   +------+-------+   +---+--+--------+
| TcpConnector +<--+ TlsConnector +-->+ UnixConnector |
+--------------+   +--------------+   +---------------+
```

Figure 1. Connectors architecture
