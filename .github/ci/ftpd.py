from pyftpdlib.authorizers import DummyAuthorizer
from pyftpdlib.handlers import FTPHandler
from pyftpdlib.servers import FTPServer
a=DummyAuthorizer(); a.add_user("jozef","secret","/tmp/ftproot",perm="elradfmwMT")
h=FTPHandler; h.authorizer=a; h.passive_ports=range(30000,30100)
FTPServer(("127.0.0.1",2121),h).serve_forever()
