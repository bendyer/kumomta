# System Preparation

Now that we know **_what_** to build, lets go ahead and build it.

* Open up AWS, select EC2 and hit the **Launch Instance** button.
* Give this server a name and then search for "rocky" in the OS images.  When you have found it, select "Rocky 9".
* Under Instance Type, select a t2.xlarge, provide your keypair for login or create a pair if necessary.
* In the Network Settings, select or create a security group that includes ports 22,25,80,443,587,2025. These will be important for sending and receiving email in a number of ways.
* Finally, modify the storage volume to 300Gb (or anything over 100Gb) and click **Launch Instance.**

... wait ....

When AWS has finished building your server instance, you can select it and connect. I prefer to find the SSH client information and use a remote terminal emulator like Putty or Terminal like this:

```console
ssh -i "yourkeyname.pem" rocky@ec2-\<pub-lic-ip\>.us-west-2.compute.amazonaws.com
```

## Doing the basics

Regardless of what system you deploy, there are things you need to do to prepare the OS before installing the MTA.

* Update to the latest patches
* Install basic testing and support tools
* Turn off services that are wasteful or can interfere
* Tune the use of memory and file access for best performance
* Automate updates and startup for resiliency

### Rocky Linux Example

Rocky Linux is very similar to CentOS, as is Alma and RHEL  The instructions below are shown for a Rocky 9 system but with slight modification, should work for any DNF package management system. For Amazon Linux (AL2) the instructions are identical, but replace "dnf" with "yum".

```console
# Do basic updates
sudo dnf clean all
sudo dnf update -y

# Grab some handy tools
sudo dnf install -y wget bind bind-utils telnet firewalld

sudo systemctl start named
sudo systemctl enable named
```

It is always a good idea to automate daily systems updates.

```console
# Make sure it all stays up to date
# Run a dnf update at 3AM daily
echo "0 3 * * * root /usr/bin/dnf update -y >/dev/null 2>&1" | \
 sudo tee /etc/cron.d/dnf-updates >/dev/null
```

... and configure the local firewall...
```console
# Build a basic firewall
sudo echo "ZONE=public
" | sudo tee -a /etc/sysconfig/network-scripts/ifcfg-eth0

sudo systemctl stop firewalld
sudo systemctl start firewalld.service
sudo firewall-cmd --set-default-zone=public
sudo firewall-cmd --zone=public --change-interface=eth0
sudo firewall-cmd --zone=public --permanent --add-service=http
sudo firewall-cmd --zone=public --permanent --add-service=https
sudo firewall-cmd --zone=public --permanent --add-service=ssh
sudo firewall-cmd --zone=public --permanent --add-service=smtp
sudo firewall-cmd --zone=public --permanent --add-port=587/tcp

sudo systemctl enable firewalld
sudo firewall-cmd --reload
```

And finally, disabling unnecessary services like postfix and qpidd

```console
sudo systemctl stop  postfix.service
sudo systemctl disable postfix.service
sudo systemctl stop  qpidd.service
sudo systemctl disable qpidd.service
```

## Creating a Self-Signed Certificate

Before you continue, you should ensure that your system has a valid SSL Certificate.  If you do not have one available, a self-signed certificate is usually ok for most purposes.  You can create one like this. (Change the certificate variables before executing this)

```console
# For the certificate enter your FQDN
MYFQDN="my.company.com"

# For the certificate, what country code are you in? (CA,US,UK, etc)
CERT_CO=US

# For the certificate, what State or Province are you in? (Alberta, California, etc)"
CERT_ST="California"

# For the certificate, what city are you in? (Edmonton, Houston, etc)"
CERT_LO="Los Angeles"

# For the certificate, what is the name of your company or organization"
CERT_ORG="My Company"

# Generate private key
openssl genrsa -out ca.key 2048

# Generate CSR
openssl req -new -key ca.key -out ca.csr -subj "/C=$CERT_CO/ST=$CERT_ST/L=$CERT_LO/O=$CERT_ORG/CN=$MYFQDN/"

# Generate Self Signed Key
openssl x509 -req -days 365 -in ca.csr -signkey ca.key -out ca.crt

# Copy the files to the correct locations
sudo mv -f ca.crt /etc/pki/tls/certs
sudo mv -f ca.key /etc/pki/tls/private/ca.key
sudo mv -f ca.csr /etc/pki/tls/private/ca.csr

# If Apache HTTPD is installed, update the SSL config (ignore errors)
sudo sed -i 's/SSLCertificateFile \/etc\/pki\/tls\/certs\/localhost.crt/SSLCertificateFile \/etc\/pki\/tls\/certs\/ca.crt/' /etc/httpd/conf.d/ssl.conf
sudo sed -i 's/SSLCertificateKeyFile \/etc\/pki\/tls\/private\/localhost.key/SSLCertificateKeyFile \/etc\/pki\/tls\/private\/ca.key/' /etc/httpd/conf.d/ssl.conf

```

Now you can move on to Installing it.

